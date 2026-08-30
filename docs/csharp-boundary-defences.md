# C# boundary defences for call-scoped, high-frequency FFI

Status: **proposed.**

This document proposes four generator-level defences for the C# backend. Each converts a class
of undefined or silent behaviour into a compile error, a build error, or a returned status.
None changes the wire format or the ownership rules established in
[the borrow-marshalling design](csharp-borrow-marshalling.md).

## Objective

The backend is increasingly used for surfaces where a Rust library holds long-lived state, C#
supplies predicates and projections, and the two exchange borrowed values at high frequency
inside a single call. That shape exposes four gaps. Three are silent; one aborts the process.

The motivating workload is a facade over a resident semantic-analysis library whose
cancellation mechanism is an unwinding panic, driven from a managed script host that invokes a
callback once per element over collections of thousands. Nothing in this document is specific
to that workload, but it explains the ordering: D1 and D3 shape whether such an API can be
written safely at all.

## Evidence and boundary

The following defences already exist and are not restated as proposals.

| Concern | Mechanism | Location |
|---|---|---|
| Bindings/DLL mismatch | Static constructor calls the guard function and compares against the baked hash at load time | `crates/backend_csharp/src/pass/output/rust/fns/guard.rs` |
| Guard hash and `Version` pattern | `Version`, `Hash`, `__api_guard` registration macro | `crates/core/src/pattern/guard.rs` |
| Managed exception escaping into Rust | Generated `CallTrampoline` catches, stores `_exception`, returns `default`; `Dispose()` re-throws | generated callback classes |
| Callback data lifetime | `_data` pointer plus generated destructor | generated callback classes |
| Async callback lifetime | `AsyncCallbackGuard` | `crates/proc_macros_impl/src/service/emit.rs` |
| Panic to status conversion | `panic_to_result` | `crates/core/src/pattern/result.rs` |
| Native allocation accounting | Global allocator wrapper with live byte and allocation counters | `crates/reference_project/src/allocation_tracking.rs` |

Two boundaries on that table. The load-time guard emits only when the inventory contains a
function whose return type is `TypePattern::Version`; an inventory without one is unguarded and
reports nothing. And `panic_to_result` is applied per function by the author: the reference
project carries both a guarded and an unguarded panicking function, and the unguarded one is
representative of what a caller gets by default.

Out of scope: cross-library handle mixing. Two cdylibs that each link the same crate hold
independent state with no stable ABI between them. A per-library identity tag would catch the
accidental case and not the aliasing one. The defence is architectural — one library per world
of shared state — and no generator change substitutes for it.

## D1. `ref struct` for call-scoped borrows

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
diagnosable — the compiler names the offending construct — rather than silent.

Owned parameters, `ref T` read/write parameters, and by-value transfer are unaffected.

An open question is whether the change is unconditional or gated behind a generator option for
one release. An option would keep existing builds green and would also keep existing unsound
code compiling, so the recommendation is unconditional, with the break documented.

## D2. Reject registered functions that cannot carry a panic

### Problem

`panic_to_result` is opt-in. A registered function that panics without it unwinds out of an
`extern "C"` boundary, which aborts the process. For libraries whose normal operation includes
panicking — cancellation implemented by unwinding is the common case — this is not an edge
condition, and the failure destroys the host with no managed diagnostic.

### Design

Extend inventory validation to reject a registered function whose return type cannot represent
a panic. `Inventory::validate()` already exists as the place where registration-time invariants
are enforced, and this reuses it: the failure surfaces when the inventory is built, naming the
function, rather than at runtime as an abort.

The check is on the return type's ability to carry the state, not on the presence of
`panic_to_result` in the body — a body-level check is not available to the generator and would
be unreliable if it were.

### Open question

Whether an author may opt out for a function that genuinely cannot panic. A blanket rule is
simpler to reason about and cheap to satisfy. An opt-out attribute is more flexible and
introduces a claim the compiler cannot verify. This document does not settle it; it should be
settled before implementation, because retrofitting an opt-out is easier than removing one.

## D3. Callback first-exception retention and poisoning

### Problem

The generated trampoline is correct in the property that matters most: a managed exception
never unwinds into Rust. Two behaviours around that are wrong for bulk invocation.

First, `_exception = e` overwrites. After several failures the retained exception is the last
one, and the first — the one that explains the failure — is discarded.

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
callback-contract change — a status distinct from the value — which is a wider change than this
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

## Verification

Per defence, the required checks.

**D1** — compile-failure tests asserting that a call-scoped value cannot be assigned to a field,
captured in a lambda, boxed, stored in a collection, or held across an `await`; and a passing
test that in-call consumption still compiles and produces identical unmanaged behaviour. The
reference project's existing shared-borrow paths must continue to pass unchanged in their
in-call form.

**D2** — an inventory test registering a function with a return type that cannot carry a panic
and asserting the validation error names it; and confirmation that the reference project's
existing registrations either satisfy the rule or are updated in the same change.

**D3** — a test invoking a callback that throws on the first of many elements, asserting that
the managed delegate is not invoked again, that `Dispose()` reports the first exception, and
that the Rust-side observed values are unchanged from current behaviour.

**D4** — a test whose callback re-enters a library function while a mutable borrow is held,
asserting an error status rather than proceeding.

Completion checks for any of the four: the full `interoptopus_csharp` target including
generated C# execution, and workspace validation from the exact candidate.

### What green verification establishes

For D1, that the enumerated constructs are rejected by the compiler. For D2, that the
enumerated invalid registrations fail at inventory build. For D3 and D4, that the enumerated
sequences produce the specified statuses.

### What it does not establish

None of these gauges observes native memory safety generally. D1 constrains the generated
managed surface; it does not prevent a caller from obtaining a raw pointer by other means. D4
detects re-entrancy through generated entry points only. The allocation probes described in
[the allocation-observability design](csharp-allocation-observability.md) remain the live
source for what allocator state is and is not observed, and their stated limits around callback
closure lifetimes are not altered by this document.

## Alternatives rejected

**Documentation instead of `ref struct`.** The rule is already documented and already
unenforced. A rule that the compiler can enforce for free should be enforced.

**Automatic `panic_to_result` wrapping in `#[ffi]`.** Would change return types of registered
functions implicitly, making the FFI signature diverge from the written one. Validation reports
the same problem without changing what the author wrote.

**Per-library handle identity tags.** Catches accidental cross-library handle use, not the
aliasing case, and could imply a guarantee the ABI does not provide. Architectural separation
is the defence.

**Aborting on callback exception rather than poisoning.** Simpler, and discards the exception
detail that makes the failure diagnosable. `Dispose()` already provides a correct reporting
point.

## Compatibility summary

| Defence | Break | Detection |
|---|---|---|
| D1 | Source-breaking for callers that store call-scoped values | Compile error naming the construct |
| D2 | Build-breaking for registrations that cannot carry a panic | Inventory validation error naming the function |
| D3 | Behavioural; first exception reported instead of last | None required |
| D4 | None | Error status at runtime |

D1 and D3 are independent of each other and of D2 and D4. Any subset can land separately.
