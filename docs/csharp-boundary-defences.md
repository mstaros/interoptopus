# C# boundary defences for call-scoped, high-frequency FFI

Status: **proposed.**

This document proposes six generator-level defences for the C# backend, and one prerequisite
they depend on. Each defence converts a class of undefined, silent, or misleading behaviour
into a compile error, a build error, or a returned status. None changes the wire format or the
ownership rules established in [the borrow-marshalling design](csharp-borrow-marshalling.md).

D1-D4 were proposed first, argued from a workload that lends borrowed values per element. D5
and D6 were added after reviewing the first consumer, which lends nothing. P0 was added after
reading the validation code: two of the defences were written against a mechanism that does not
exist. The sequencing section records what that evidence changed.

## Objective

The backend is used across two boundary policies that pull in opposite directions.

One lends: a Rust library holds long-lived state, C# supplies predicates and projections, and
borrowed values cross for the duration of a single call. That policy needs the lending to be
safe.

The other detaches: nothing borrowed crosses at all, and every payload owns its data. That
policy needs the absence of lending to be provable rather than reviewed.

Interoptopus supports lending today, so both are legitimate consumer policies and the tool
should not pick one. It should let a consumer declare which policy applies and enforce it. D1
serves the first, D5 the second, and they are deliberate mirror images. D2, D3, D4 and D6 apply
under either policy.

## Evidence and boundary

The following defences already exist and are not restated as proposals.

| Concern | Mechanism | Location |
|---|---|---|
| Bindings/DLL mismatch | Static constructor calls the guard function and compares against the baked hash at load time | `crates/backend_csharp/src/pass/output/rust/fns/guard.rs`, template `templates/rust/fns/guard.cs` |
| Guard hash and `Version` pattern | `Version`, `Hash`, `__api_guard` registration macro | `crates/core/src/pattern/guard.rs` |
| Managed exception escaping into Rust | Generated `CallTrampoline` catches, stores `_exception`, returns `default`; `Dispose()` re-throws | generated callback classes, e.g. `crates/backend_csharp/benches/dotnet/Interop.Common.cs` |
| Callback data lifetime | `_data` pointer plus generated destructor | same |
| Async callback lifetime | `AsyncCallbackGuard` | `crates/proc_macros_impl/src/service/emit.rs` |
| Panic to status conversion | `panic_to_result` | `crates/core/src/pattern/result.rs` |
| Native allocation accounting | Global allocator wrapper with live byte and allocation counters | `crates/reference_project/src/allocation_tracking.rs` |
| Registration-time name and shape rules | Proc-macro validation returning `syn::Result<()>` | `crates/proc_macros_impl/src/types/validation.rs`, `crates/proc_macros_impl/src/service/validation.rs` |

Two boundaries on that table. The load-time guard emits only when the inventory contains a
function whose return type is `TypePattern::Version`; an inventory without one is unguarded and
reports nothing. And `panic_to_result` is applied per function by the author: the reference
project carries both a guarded and an unguarded panicking function, and the unguarded one is
representative of what a caller gets by default.

Out of scope: cross-library handle mixing. Two cdylibs that each link the same crate hold
independent state with no stable ABI between them. A per-library identity tag would catch the
accidental case and not the aliasing one. The defence is architectural - one library per world
of shared state - and no generator change substitutes for it.

## P0. Inventory validation does not exist yet

This is a prerequisite, not a defence. D2 and D5 both need a place to enforce a rule across a
whole inventory, and that place is currently a name without a body.

### Problem

`Inventory::validate()` performs no validation. Both the Rust and plugin inventories implement
it as a swap-and-return:

```rust
pub fn validate(&mut self) -> Self {
    let mut rval = Self::new();
    swap(&mut rval, self);
    rval
}
```

`crates/core/tests/inventory/forbidden.rs` carries a commented-out `#[should_panic]` test
asserting that a forbidden function name is rejected through `validate()`, so inventory-level
validation was intended and did not land.

What does exist is proc-macro validation. `types/validation.rs` and `service/validation.rs`
return `syn::Result<()>`, so forbidden names and shape rules are rejected at compile time with a
span pointing at the offending item. That is better diagnostics than an inventory-time check,
not worse.

An earlier revision of this document asserted that `Inventory::validate()` "already exists as
the place where registration-time invariants are enforced". That was incorrect and is retracted
here.

### Why the proc-macro layer is not sufficient for D2 and D5

The proc macro sees one item at a time. It cannot see a policy declared elsewhere, and it
cannot answer a question about the whole registered surface. A rule such as "this library
lends nothing" is a property of an inventory, not of any single item, so it cannot be enforced
where the current validation lives.

### Design

Give `Inventory::validate()` a real body and a real failure mode, then express D2 and D5 as
rules over the assembled inventory.

Policy is declared per inventory rather than per registration. That matches how a consumer
states such a rule - as a property of the whole library - and it prevents the invariant being
defeated one attribute at a time. A per-registration opt-out, if it is wanted at all, is a
later addition and should be argued separately.

### Open question - failure mode

`validate()` currently returns `Self`. Making it reject anything requires either a panic or a
signature change to a `Result`.

The commented-out test expects a panic, which suggests panic was the original intent and is the
non-breaking route. Against that: a build-time invariant that aborts the build process is worse
to diagnose than one that reports, and callers cannot handle it.

Returning `Result` is the better contract and is a breaking change to a public API on this fork.
This document does not settle it. It should be settled before P0 is implemented, because both
D2 and D5 inherit whichever is chosen.

## D1. `ref struct` for call-scoped borrows

Applies under a lending policy. Deferred; see sequencing.

### Problem

The `in T` parameter mode passes a read-only pointer whose pointee is valid only for the
duration of the call. Nothing in the generated C# expresses that. A caller may assign the value
to a field, capture it in a lambda that outlives the call, box it, place it in a collection, or
hold it across an `await`. Each is a use-after-free with no diagnostic. The rule exists today
only as prose.

### Design

Emit the managed side of a call-scoped borrow as a `ref struct`. The C# compiler then rejects
field assignment, boxing, capture in a lambda or local function, use as a generic type argument,
storage in an array or collection, and any live use across an `await` or `yield`. The entire
class becomes compile errors at the call site, with no runtime cost and no change to the
unmanaged representation.

This is the strongest defence available for this class because it is the only one that does not
depend on the caller following a rule.

### Compatibility

Breaking for any existing caller that stores a call-scoped value rather than consuming it
within the call. That is the intended effect: such code is already unsound. The break is
diagnosable - the compiler names the offending construct - rather than silent.

Owned parameters, `ref T` read/write parameters, and by-value transfer are unaffected.

An open question is whether the change is unconditional or gated behind a generator option for
one release. An option would keep existing builds green and would also keep existing unsound
code compiling, so the recommendation is unconditional, with the break documented.

## D2. Reject registered functions that cannot carry a panic

Depends on P0.

### Problem

`panic_to_result` is opt-in. A registered function that panics without it unwinds out of an
`extern "C"` boundary, which aborts the process. For libraries whose normal operation includes
panicking - cancellation implemented by unwinding is the common case - this is not an edge
condition, and the failure destroys the host with no managed diagnostic.

### Design

Reject a registered function whose return type cannot represent a panic, as a rule over the
assembled inventory once P0 provides a place to put it. The failure names the function.

The check is on the return type's ability to carry the state, not on the presence of
`panic_to_result` in the body - a body-level check is not available to the generator and would
be unreliable if it were.

A proc-macro-time variant is possible for the subset of cases where the return type alone
decides it, and would give better spans. It is not sufficient on its own if the rule is ever
made conditional on a declared policy, which is why this is stated against P0.

### Open question

Whether an author may opt out for a function that genuinely cannot panic. A blanket rule is
simpler to reason about and cheap to satisfy. An opt-out attribute is more flexible and
introduces a claim the compiler cannot verify. This document does not settle it; it should be
settled before implementation, because retrofitting an opt-out is easier than removing one.

### Sequencing constraint

D2 constrains the return-type shape of every registered function. A consumer whose error
envelope is not yet frozen would have that shape constrained by this rule and then change it,
producing two migrations instead of one. D2 should not land ahead of a consumer's error-envelope
decision.

## D3. Callback first-exception retention and poisoning

Applies where a managed delegate is invoked per element from Rust. Deferred; see sequencing.

### Problem

The generated trampoline is correct in the property that matters most: a managed exception
never unwinds into Rust. Two behaviours around that are wrong for bulk invocation.

First, `_exception = e` overwrites. After several failures the retained exception is the last
one, and the first - the one that explains the failure - is discarded.

Second, there is no poisoning. After a failure the trampoline keeps invoking the managed
delegate for every remaining element. Rust receives `default` each time. For a predicate,
`default` is `false`, so a failing predicate reports "no match" for the whole collection while
throwing once per element, and the caller learns of it only at `Dispose()`. Over a large
collection this is both a large hidden cost and a misleading result.

### Design

Retain the first exception rather than the last: assign only when no exception is pending.

Add a poisoned flag set on first failure. While poisoned, the trampoline returns `default`
immediately without invoking the managed delegate. This removes the repeated throw cost and
makes the post-failure behaviour uniform.

`Dispose()` continues to re-throw, now reporting the first exception.

### Boundary

This does not stop the Rust-side loop. Rust still iterates the full collection and still
observes `default` for each remaining element. Signalling abort to the caller requires a
callback-contract change - a status distinct from the value - which is a wider change than this
document proposes and is deliberately left out. The two changes here are independent of it and
do not conflict with it.

### Compatibility

Behavioural, not source-breaking. Code that relied on observing the last exception rather than
the first would change; that reliance is not a supported contract.

## D4. Re-entrancy guard

### Problem

A managed callback invoked while the Rust side holds a mutable borrow may call back into a
library function that takes the same borrow. That is aliasing undefined behaviour. Nothing
detects it.

### Design

A thread-local depth counter set while a mutable borrow is held across a callback invocation,
checked at every generated library entry point. An entry observed while the flag is set returns
an error status instead of proceeding.

Cost is one thread-local read per call, which is not material relative to the marshalling
already performed. The result is a diagnosable failure rather than corruption.

## D5. Reject borrowed registrations under a detach-only policy

The mirror of D1. Where D1 makes lending safe to use, D5 makes the absence of lending provable.
Depends on P0.

### Problem

A consumer may adopt a policy that no borrowed data crosses the boundary at all: every payload
owns or detaches its data before it is returned, and lifetime-bound types are never exposed.
That policy is sound and simple, but today it is enforced only by review. As surface area
grows, a single registration that returns a borrowed view reintroduces the entire hazard class
silently, and the reviewer who would have caught it is looking at a diff, not at an invariant.

This is not hypothetical. The first consumer states the rule as a required invariant, records
that it needs automated enforcement rather than convention, and explicitly leaves the mechanism
unchosen. It repeats the same requirement for its streaming payloads: violations should become
a build or test failure rather than a review convention.

### Detection surface

This defence cannot be stated as "reject lifetimes". Lifetime-carrying types are first class in
this codebase and deliberately so:

- `ffi::Slice<'a, T>` and `ffi::SliceMut<'a, T>` are the canonical borrowed types
  (`crates/core/src/pattern/slice.rs`), and the borrow-marshalling design keeps their existing
  copy/borrow semantics unchanged.
- `crates/proc_macros_impl/src/function/emit.rs` explicitly handles `syn::GenericParam::Lifetime`
  and synthesizes `PhantomData` for it.
- `crates/core/tests/ui/proc/svc/lifetime.rs` is registered as a **pass** test: a service method
  with `<'a, 'b>` taking two `ffi::Slice` parameters compiles today by design.

So the rule is not about lifetimes as a language feature. It is about a declared policy that a
particular inventory exposes no borrowed data, and its detection surface is the specific
patterns that carry a borrow across the boundary: `Slice` and `SliceMut`, read-only and
read/write pointer-family parameters, and user types carrying a lifetime parameter.

### Design

Under a declared detach-only policy, reject any registration whose signature or exposed types
match the detection surface above. The failure names the type or function.

The policy is a declaration rather than a default, because lending is a supported mode and D1
exists to make it safe. An inventory that declares nothing keeps current behaviour, including
the passing lifetime test above.

### Boundary

The check is sound against the common case and is not a proof. A type that owns a raw pointer
into borrowed data is indistinguishable from one that owns its buffer, and the generator cannot
tell them apart. The document should claim detection of declared borrows, not absence of
aliasing.

### Compatibility

No effect on a consumer that does not declare the policy. For one that does, build-breaking by
construction, which is the requested behaviour.

## D6. Do not claim ownership where none exists

### Problem

Every generated `Result<T, E>` projection is emitted as a class implementing `IDisposable`,
carrying a remark that the value is an owned resource which the caller must dispose, move, or
free.

For a large share of instantiations that claim is false. A Result whose `Ok` payload is a
primitive and whose `Err` payload owns nothing has nothing to dispose. The generated code
allocates a heap object per conversion to carry it, and imposes a disposal obligation on every
caller that is meaningless to satisfy.

The allocation itself is minor and should not be the argument. Measured on a 64 MiB streaming
read by the first consumer, per-call result construction accounted for 1,026 allocations
totalling roughly 41 KB over a 105 ms read - a gen0 rate low enough to be irrelevant, in a
workload the same measurement shows is I/O bound. The defect is the false ownership claim, not
its cost.

### Design

Split the projection by whether the payload actually owns an unmanaged resource.

Neither `Ok` nor `Err` owns a resource: emit a value type, without `IDisposable`, without the
ownership remark.

Either owns a resource: keep the value non-copyable and the disposal obligation, because a
copied handle followed by two disposals is a double free. The remark is then accurate.

### Open question

The generated Results implement a public `IResult<out T, out TErr>` interface. A value-type
Result boxes when converted to that interface, which would move the allocation rather than
remove it. In the first consumer the interface has no consumption sites - it is declared,
implemented on every Result, and held by nothing - so the change is safe to attempt there. That
is evidence about one consumer and not a general licence: another may hold Results through the
interface.

The generic options are to keep the interface and document the boxing, to stop emitting it for
non-owning Results, or to make its implementation opt-in. This should be decided before
implementation, not discovered by a consumer.

## Priority and sequencing

The original ordering placed D1 first, argued from a lending workload. Two later reviews changed
the basis for that argument.

**No consumer currently lends.** The first consumer's stated design rule is that no borrowed
data crosses the boundary; its record streams use managed pull enumeration with Rust-owned
producer state, not per-element managed callbacks. So neither D1 nor D3 has a consumer
exercising it, and their compile-failure and poisoning tests would be written against the
reference project alone.

**The cost that lending would avoid was measured and is small.** The first consumer's byte
streaming benchmark performs one native-to-managed copy per read. The copy runs at
approximately memcpy speed and accounts for roughly three percent of a realistic I/O-bound
read. Lending flat payloads within a call would recover about that much, in exchange for
reintroducing the hazard class D1 exists to contain. That trade does not currently favour
lending, and the rule against it therefore rests on measurement rather than on inheritance from
the object-handle and streaming cases where it is independently justified.

**Two defences were written against a mechanism that does not exist.** D2 and D5 both assumed an
inventory-level validation step. P0 has to land first, and its failure-mode question has to be
answered first, because both inherit the answer.

This does not make D1 or D3 wrong. It defers them until a consumer adopts a lending policy or a
per-element callback surface, at which point they become prerequisites rather than improvements.

Recommended order:

1. **P0** - prerequisite for D2 and D5; settle the failure mode before writing it.
2. **D5** - requested twice by the first consumer, currently unowned, and the enforcement
   mechanism is explicitly open.
3. **D4** - independent of every policy question and cheap.
4. **D6** - a correctness defect in the generated projection, independent of the boundary
   policy, pending the `IResult` decision.
5. **D2** - after a consumer's error envelope is frozen, per the sequencing constraint above.
6. **D1**, **D3** - when a lending or per-element-callback consumer exists.

## Verification

Per defence, the required checks.

**P0** - a test asserting that an inventory containing a known-invalid registration fails, in
whichever failure mode is chosen; and re-enabling the commented-out forbidden-name test in
`crates/core/tests/inventory/forbidden.rs` as the first rule expressed through the new
mechanism.

**D1** - compile-failure tests asserting that a call-scoped value cannot be assigned to a field,
captured in a lambda, boxed, stored in a collection, or held across an `await`; and a passing
test that in-call consumption still compiles and produces identical unmanaged behaviour.

**D2** - an inventory test registering a function with a return type that cannot carry a panic
and asserting the failure names it; and confirmation that the reference project's existing
registrations either satisfy the rule or are updated in the same change.

**D3** - a test invoking a callback that throws on the first of many elements, asserting that
the managed delegate is not invoked again, that `Dispose()` reports the first exception, and
that the Rust-side observed values are unchanged from current behaviour.

**D4** - a test whose callback re-enters a library function while a mutable borrow is held,
asserting an error status rather than proceeding.

**D5** - an inventory test declaring a detach-only policy and registering an `ffi::Slice`
parameter, asserting the failure names it; and confirmation that
`crates/core/tests/ui/proc/svc/lifetime.rs` still passes unchanged when no policy is declared.

**D6** - allocation-probe tests asserting that a non-owning Result conversion allocates nothing
on the managed heap; a compile-failure or runtime test that an owning Result cannot be disposed
twice; and confirmation that no generated non-owning Result carries the ownership remark.

Completion checks for any of these: the full `interoptopus_csharp` target including generated
C# execution, and workspace validation from the exact candidate.

### What green verification establishes

For D1, that the enumerated constructs are rejected by the compiler. For P0, D2 and D5, that the
enumerated invalid registrations fail at inventory build. For D3 and D4, that the enumerated
sequences produce the specified statuses. For D6, that the enumerated conversions do not
allocate and that the ownership remark tracks actual ownership.

### What it does not establish

None of these gauges observes native memory safety generally. D1 constrains the generated
managed surface; it does not prevent a caller from obtaining a raw pointer by other means. D4
detects re-entrancy through generated entry points only. D5 detects borrows declared in a
registered signature; a type that owns a raw pointer into borrowed data is not distinguishable
by the generator, so D5 is sound against the common case and is not a proof. The allocation
probes described in [the allocation-observability design](csharp-allocation-observability.md)
remain the live source for what allocator state is and is not observed, and their stated limits
around callback closure lifetimes are not altered by this document.

## Alternatives rejected

**Documentation instead of `ref struct`.** The rule is already documented and already
unenforced. A rule that the compiler can enforce for free should be enforced.

**Automatic `panic_to_result` wrapping in `#[ffi]`.** Would change return types of registered
functions implicitly, making the FFI signature diverge from the written one. Validation reports
the same problem without changing what the author wrote.

**Expressing D5 at proc-macro time only.** Better spans, but the proc macro sees one item at a
time and cannot observe a policy declared for the inventory. A whole-surface invariant needs a
whole-surface check.

**Rejecting lifetime-carrying types outright.** Would break `ffi::Slice`, the pointer-family
parameter modes, and an existing passing UI test. Lending is a supported mode; the policy is
what varies, not the language feature.

**Per-library handle identity tags.** Catches accidental cross-library handle use, not the
aliasing case, and could imply a guarantee the ABI does not provide. Architectural separation
is the defence.

**Aborting on callback exception rather than poisoning.** Simpler, and discards the exception
detail that makes the failure diagnosable. `Dispose()` already provides a correct reporting
point.

**Making detach-only the default instead of a declared policy.** Lending is a supported mode
with an existing design; defaulting it off would break it silently for anyone using it. The
policy is a declaration precisely so that both policies remain first class.

**Making every Result a value type.** Correct only where the payload owns nothing. A copyable
Result carrying an owned handle turns one disposal obligation into an unbounded number of them.

## Compatibility summary

| Item | Break | Detection |
|---|---|---|
| P0 | `Result` return would break `validate()` callers; panic would not | Depends on the chosen failure mode |
| D1 | Source-breaking for callers that store call-scoped values | Compile error naming the construct |
| D2 | Build-breaking for registrations that cannot carry a panic | Inventory failure naming the function |
| D3 | Behavioural; first exception reported instead of last | None required |
| D4 | None | Error status at runtime |
| D5 | Build-breaking, only for a consumer that declares the policy | Inventory failure naming the type |
| D6 | Source-breaking where callers dispose a now non-owning Result | Compile error on the removed `Dispose()` |

D1 and D3 are independent of each other and of the rest. D2 and D5 both depend on P0. D5 is
independent of D1 despite being its mirror: a consumer declares one policy or the other, and the
tool supports both. D6 is independent of all of them, pending its own open question.
