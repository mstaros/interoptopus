# C# 15 union projection for Rust enums

Status: **plan, not approved.** One blocking decision open (Step 0). No code written.

Scope: project `#[ffi]` Rust enums as C# 15 custom unions, opt-in, native ABI unchanged.
`ffi::Option` / `ffi::Result` are designed for but deliberately out of scope for the first
implementation.

---

## 1. Representation: custom `[Union]`, not the `union` keyword

The `union` keyword was evaluated and rejected. Three reasons, in order of weight:

**Storage.** A `union` declaration lowers to a struct whose entire storage is a single
`object? Value` auto-property. Every payload-carrying enum crossing the FFI boundary would
box. The current tagged representation (`_variant` + inline payload fields) is
allocation-free and *is* the wire format.

**Instance fields are forbidden in a union declaration body.** `definition.cs` emits
`_variant` and one field per payload variant. Those cannot exist in a keyword union, so the
existing layout could not be preserved even if boxing were acceptable.

**The keyword always produces a struct.** `model/common/types/info/struct_class.rs`
deliberately emits some owned types as classes (`OptionUtf8String`, owned `Result` types).
The keyword cannot express that; a custom `[Union]` type may itself be a class.

The specification explicitly sanctions hand-coding for exactly these reasons: *"Adapting
existing types to the union patterns to gain union behaviors"* and *"Implementing a
different storage strategy for e.g. efficiency or interop reasons."*

### Case types

C# unions are unions *of types*. A discriminated union is expressed by giving each case its
own fresh type — this is the documented idiom, and the specification's own example:

```csharp
public record class None();
public record class Some<T>(T value);
public union Option<T>(None, Some<T>);
```

So each Rust variant gets one nested `readonly record struct`. This removes two apparent
problems that do not actually arise: unit variants get an empty case type, and two variants
sharing a payload type (`E { A(u32), B(u32) }`) get two distinct case types rather than
colliding.

### Resulting consumer API

```csharp
EnumPayload x = EnumPayload.B(vector);          // existing factory, preserved
EnumPayload y = new EnumPayload.BCase(vector);  // union conversion

var result = x switch
{
    EnumPayload.ACase           => HandleA(),
    EnumPayload.BCase(var vec)  => HandleB(vec),
    EnumPayload.CCase(var num)  => HandleC(num),
};                                               // exhaustive, no default arm
```

Pattern matching binds through `TryGetValue`, so ordinary matches do not allocate.

---

## 2. Prerequisites

**Step 0 is filed as `Issues.md` `09b82d44`** (discriminant resolution). Defect 1 is confirmed
by execution; see that issue. It blocks item 4a but is a correctness bug in its own right.

**Snapshot baseline.** Effectively green. `Issues.md` `ccb105a2` measures **30 passed,
1 failed, 2 ignored** of 33 on `b42399a4`; the single failure is `reference_project::interop`,
awaiting `cargo insta review` for an unrelated `AsSpan()`/`ToArray()` template change.

An earlier version of that issue reported 29 of 33 failing and was cited in earlier drafts of
this plan as a hard blocker. **That was retracted** — it came from a Git LFS materialization
failure, not from the repository.

**LFS in transaction worktrees — earlier warning retracted.** A previous revision of this
section claimed MCP transaction worktrees do not materialize LFS content, that the Step 5
regression gate was therefore unusable inside one, and that snapshot work belonged in the real
checkout. **Measured 2026-08-25 and false.** In worktree `086e4102`,
`r#mod__reference_project__interop.snap` is 848,675 bytes — byte-for-byte the same size as the
main checkout. `reference_project::interop` produced a real content diff, not the
*"snapshot uses a legacy snapshot format"* error that a pointer stub yields.

So snapshot-driven work, including Step 5, can run inside a transaction. No split between
worktree and real checkout is needed.

What remains true and unmeasured: `.gitattributes` tracks `*.snap` and `*.dll` in LFS, so a
fresh clone without `git lfs pull` or a CI checkout without LFS support will still see pointer
stubs, and `cargo insta review` in that state is destructive — accepting against stub baselines
overwrites the pointers with raw content. Whether that was `ccb105a2`'s original environment is
unknown. Cheap guard before measuring anywhere unfamiliar: confirm that snapshot is ~847 KB and
not three lines.

**Toolchain.** Union consumers need `<LangVersion>preview</LangVersion>` and a .NET 11
Preview 5+ runtime for `UnionAttribute` / `IUnion`.
`tests/reference_project/Bindings/Bindings.csproj` targets `net10.0`. The feature must
therefore be opt-in; flag-off output stays byte-identical and net10 consumers are unaffected.

---

## Step 0 — Discriminant becomes an attribute of the variant

**This is a correctness fix that stands alone. It is not union groundwork, and should be
reviewed as a bug.**

`crates/backend_csharp/src/pass/model/common/types/kind/enum_variants.rs`:

```rust
VariantKind::Unit(tag)          => (*tag, None),
VariantKind::Tuple(rust_type_id) => (index.cast_signed(), Some(cs_type_id)),
```

Unit variants carry their declared discriminant; payload variants carry their positional
index. The backend has no alternative — `crates/core/src/lang/types/enums.rs` defines:

```rust
pub enum VariantKind {
    Unit(isize),
    Tuple(TypeId),
}
```

`Tuple` carries no discriminant at all. The information does not exist in the core model.

**Consequence today, without unions:** for any mixed enum with explicit discriminants — e.g.
`enum E { A = -1, B(u32) }` — managed `_variant` and the native Rust tag disagree. `A` gets
`-1`, `B` gets positional `1`, and Rust assigns `B` whatever its own rules produce.

**Consequence for this plan:** every downstream step assumes `_variant` is the native
discriminant. Step 4's validated `ToManaged` switch would validate against the wrong tag set.
Step 0 must land first.

### Proposed change

Move the discriminant onto `Variant`, leaving `VariantKind` as a pure payload descriptor:

```rust
pub struct Variant {
    pub name: String,
    pub docs: Docs,
    pub tag: isize,
    pub kind: VariantKind,
}

pub enum VariantKind {
    Unit,
    Tuple(TypeId),
}
```

### Blast radius

| Location | Change |
|---|---|
| `core/src/lang/types/enums.rs` | struct + `VariantKind` + `Variant::new` signature |
| `proc_macros_impl` | must compute discriminants (see below) |
| `backend_csharp/.../enum_variants.rs` | read `v.tag` unconditionally |
| `backend_c`, `backend_cpython` | defunct but still compile against these types |
| serde inventory shape | changes under the `serde` feature |

The real work is in the proc macro: it must replicate Rust's discriminant assignment —
explicit values where given, previous-plus-one otherwise — across mixed unit and payload
variants. **Unverified:** whether it currently sees explicit discriminants on payload
variants at all.

### OPEN DECISION

`tag` as a field on `Variant`, or a second element in `VariantKind::Tuple`?

Recommendation: **field on `Variant`.** It makes the discriminant unconditional and prevents
the same divergence recurring when named and multi-field variants are added. Cost: it changes
a public core type and the `Variant::new` signature.

---

## Step 1 — `union_projection` model pass

New pass under `model/common/types/enums/`. It is the single source of truth for both
*enablement* and *naming*.

**Enablement is eligibility.** The pass holds an entry only for a `DataEnum` that is both
enabled and eligible. Output passes ask "does this type have a union projection?" — there is
no `unions: bool` copied into six or eight output-pass configs, so inconsistent state is
unrepresentable. This simultaneously gates the feature and excludes `Option` / `Result`,
which matters because the existing enum passes all match `DataEnum`, `Result` and `Option`
together.

### Naming

Every generated member derives from one collision-free **stem** per variant:

| Member | Form |
|---|---|
| factory | `{Stem}` |
| case type | `{Stem}Case` |
| check | `Is{Stem}` |
| accessor | `As{Stem}` |
| managed field | `_{Stem}` |
| unmanaged helper type | `Unmanaged{Stem}` |
| unmanaged field | `_{Stem}` |

**The stem is the currently-emitted name, not a re-cased one.** Templates use `v.name`
verbatim today. Unconditionally pascal-casing would rename existing public members on enums
that have no collision at all — a silent breaking change. Casing is applied only as part of
collision resolution.

`names.rs` is not the right home: it is keyed `TypeId -> String`, one name per *type*, and
has no notion of member names inside a type.

### Reserved names

`Value`, `HasValue`, `Unmanaged`, `Marshaller`, `MarshallerMeta`, `ToUnmanaged`,
`AsUnmanaged`, `ToManaged`, `ToString`, `Dispose`, `ExceptionForVariant`, `TryGetValue`,
**and the enclosing enum's own name** (CS0542: a member may not have the same name as its
enclosing type).

### Collision classes

1. **Case type vs. member.** A nested type may not share a name with a non-type member in the
   same declaration (CS0102). This is why the `Case` suffix exists: it lets
   `EnumPayload.BCase` coexist with the existing `EnumPayload.B(...)` factory.
2. **Fold collisions.** Identifiers that sanitize or case-fold to the same C# identifier —
   `foo_bar` and `FooBar` both reach `FooBar` through the existing
   `interoptopus_backends::casing` helpers.
3. **Cross-family collisions.** A variant named `B` and a variant named `IsB`.
4. **Reserved-set collisions.** A variant named `Value` or `HasValue` produces a factory that
   clashes with a mandatory union member. This is *new* — no `Value` property exists today,
   so union mode creates the conflict.

**Policy: prefer disambiguating the new case type over renaming an existing public member.**
`IsX`, `AsX` and the factory are today's API; the case type is new. Class 4 is the exception —
there the factory itself is the conflict and must move.

Appending a disambiguator converges over a finite set. A defensive iteration limit is
sufficient; a "cannot converge" error is not part of the contract.

### Templates that consume raw `v.name` and must be migrated

`definition.cs`, `body_ctors.cs`, `body_unmanaged.cs`, `body_unmanaged_variant.cs`,
`body_tostring.cs`, `body_exception_for_variant.cs`, `body.cs` (`disposable_variants`).

---

## Step 2 — Builder flag

`RustLibraryBuilder::unions(bool)`, default off, consumed by the projection pass.
`RustLibraryConfig` is internal; `RustLibraryBuilder` is the public surface.

**Verified.** `pipeline/rust/builder.rs` exposes `#[must_use]` fluent setters that each write
into `self.config.<pass_config>.<field>` — `dispatch`, `dll_name`, `headers`, `search_path`.
The projection pass therefore gets a `Config { enabled: bool }` field on `RustLibraryConfig`,
set the same way:

```rust
/// Enables C# 15 union projection for eligible enums.
#[must_use]
pub fn unions(mut self, enabled: bool) -> Self {
    self.config.model_union_projection.enabled = enabled;
    self
}
```

This does not contradict "one source of truth": the config field only seeds the pass. Output
passes query the *pass*, never the config.

---

## Step 3 — Managed representation

Struct-backed and class-backed unions have **different contracts**. They are not unified.

### Struct-backed

Storage: existing `_variant` + payload fields, plus `bool _hasValue` in the managed partial
**only**. `_hasValue` never appears in `Unmanaged`.

| Operation on `default(E)` | Behaviour |
|---|---|
| `Value` | `null` |
| `HasValue` | `false` |
| `IsX` | `false` |
| `TryGetValue(out …)` | `false` |
| `AsX()` | throws (empty-state exception) |
| `Dispose()` | no-op |
| `ToUnmanaged()` / `AsUnmanaged()` | throws |
| `ToString()` | `"<empty>"` |

Every managed tag consumer must respect `_hasValue` — not only `Dispose()`.

### Class-backed

**There is no empty class instance.** `default(E)` is a null reference; no member is callable
on it. Every non-null instance is valid.

- No `_hasValue` field.
- `HasValue => true` (constant).
- `Value` is never null.
- A **private parameterless constructor** replaces today's implicit public one, so
  `new EnumX()` can no longer produce a bogus variant-zero instance. Factories, case
  constructors and `Unmanaged.ToManaged()` construct from inside the type and are unaffected.

**Flag-on breaking change:** external `new EnumX()` stops compiling for class-backed enums.
Changelog entry required.

**OPEN:** what happens when `null` reaches the custom marshaller — today's
`NullReferenceException`, or a deliberate `ArgumentNullException` / `InteropException`?

### `Value`

Materialise the case wrapper on access:

```csharp
public object? Value => !_hasValue ? null : _variant switch
{
    0 => new ACase(),
    1 => new BCase(_B),
    _ => throw /* empty-state or corruption, per Step 4 */,
};
```

**No `_boxed` field.** An eagerly-populated field would allocate inside `ToManaged`, i.e. on
every boundary crossing — the exact cost this representation exists to avoid. Lazy caching
does not rescue it, because struct copies would each re-box.

Reference identity is *not* stable across `Value` accesses for payload cases. This does not
violate the specification: soundness, stability and creation-equivalence constrain the
*type* of `Value`, not its identity. `Value` is the fallback path; patterns use
`TryGetValue`. Document as **value-stable, not reference-stable**.

A cached static per payload-free case is a possible later optimisation. It removes
per-access allocation, not all allocation, and it needs a name of its own — which feeds back
into the naming surface above. **Excluded from the first implementation.**

### `TryGetValue`

```csharp
public bool TryGetValue(out BCase value)
{
    if (_hasValue && _variant == 1) { value = new BCase(_B); return true; }
    value = default;
    return false;
}
```

### Interface list

`body.cs` currently composes its base list through nested conditionals for `IResult` and
`IDisposable`. Adding `IUnion` makes that fragile. Build a `Vec<String>` and render
`: {{ interfaces | join(sep=", ") }}`.

---

## Step 4 — Conversions

Native `Unmanaged` layout is unchanged. The conversion *paths* are not.

**`ToUnmanaged()` / `AsUnmanaged()`** reject the empty struct state before copying. One
branch, not a translation table.

**`Unmanaged.ToManaged()` must construct, not mutate.** Today it does:

```csharp
var _managed = new EnumPayload();
_managed._variant = _variant;
```

with no tag validation. Replace with a validated switch returning through the case
constructors:

```csharp
return _variant switch
{
    0 => new EnumPayload(new EnumPayload.ACase()),
    1 => new EnumPayload(new EnumPayload.BCase(_B._B.ToManaged())),
    _ => throw new InteropException(...),
};
```

This works identically for structs and classes, establishes `_hasValue` implicitly, and makes
an invalid native tag unrepresentable as a managed value. For struct constructors, mind
definite assignment — `this = default` before assigning tag, payload and `_hasValue`.

**These switch arms are keyed on `_variant`. Step 0 must land first or they validate against
the wrong tag set.**

### Exceptions

A default struct union is a legal C# state, not corruption. `InteropException` in this
codebase means *"severe error, should never happen"*.

| Failure | Exception |
|---|---|
| empty `AsX()` / marshal-out | `InvalidOperationException` |
| unknown native discriminant | `InteropException` |

`ExceptionForVariant()` returns the empty-state exception so existing
`throw ExceptionForVariant()` call sites stay coherent.

---

## Step 5 — Tests

**Not tested:** `[Union]` recognition, exhaustive matching, implicit union conversions. Those
are compiler features.

**Tested — our behaviour:**

- flag-off snapshots byte-identical (the regression gate)
- one focused flag-on union snapshot
- **flag-on output compiles** under a small net11 / `LangVersion=preview` fixture. This
  proves the generator emits legal C#; it asserts nothing about union semantics. Currently
  nothing compiles union-mode source: `reference_project::interop` only writes and compares
  text, and `Bindings.csproj` is net10. (`Tests/InteropSpike.csproj` is net11 but reportedly
  references the net10 bindings project — **unverified**.)
- variant named `Value`
- casing-fold collision
- `B` / `BCase`, and cross-family `B` / `IsB`
- `default(struct E).ToUnmanaged()` throws
- class-backed union cannot produce a non-null empty instance
- invalid native tag throws
- default disposable struct `Dispose()` is a no-op
- one managed-only `DataEnum` (`body.rs` supports `DataEnum`s with no `Unmanaged` form)
- existing Rust round trip unchanged

---

## Step 6 — `Option` / `Result`

Same representation: `NoneCase` / `SomeCase`, `OkCase` / `ErrCase`. Deferred until the plain
case is proven. Constraints to preserve now:

- `Result` implements `IResult<T,E>` with `AsOk()` / `AsErr()` and unit-side methods. Case
  types must coexist with that interface, not replace it.
- `default(ResultX)` must be **empty**, not `Ok`.
- `default(OptionX)` must be distinct from `NoneCase`.

Retiring `IsOk` / `AsOk` is a separate breaking change and is not in scope.

---

## Output routing

Same generated file. **Verified** against `pass/output/common/master.rs`: it builds
`type_routing: HashMap<TypeId, Target>` by classifying every type reachable from
`types::all` through the configured `Dispatch`. Only registered `TypeId`s are classified.

Nested case types have no `TypeId` and are absent from `types::all`, so they are never
classified and simply render inside the parent enum's body. No registration, no separate
routing.

This hardens the corollary: case types must **not** be added to `types::all` or given FFI
`TypeId`s. Beyond the fallback pass trying to map them, they would become independently
routable and could be dispatched to a *different output file* than their parent — which for
a nested declaration is a compile error, not merely wasted work.

---

## Todo

Execution state. Rationale for every item lives in the step sections above; this table tracks
only what is done. Step 0 is tracked as `Issues.md` `09b82d44`, not here.

| # | Status | Item | Gate |
|---|---|---|---|
| 0 | blocked | Discriminant onto `Variant` (`09b82d44` defect 2) | **Open decision: `Variant.tag` vs `VariantKind::Tuple(_, tag)`** |
| 0a | open | Proc macro: `next_discriminant = resolved + 1` (`09b82d44` defect 1) | none — independent, ships alone |
| 0b | open | Proc macro: read `variant.discriminant` in the `Tuple` arm | 0 |
| 0c | open | 3 index-as-tag sites: `enum_variants:54`, `wire:318`, `wire:352` (defect 3) | 0b |
| 0d | open | Reference enum `{ A = 5, B, C }` → confirms/kills defect 1 | none — do first |
| 0e | open | Reference enum mixing explicit discriminants + payload variants | 0c |
| 1 | open | `union_projection` model pass: eligibility + name resolution | — |
| 1a | open | Reserved-name set incl. enclosing type name (CS0542) | 1 |
| 1b | open | Stem = currently-emitted name, not re-cased | 1 |
| 1c | open | Migrate 7 templates off raw `v.name` | 1 |
| 2 | open | `RustLibraryBuilder::unions(bool)` | 1 |
| 3 | open | Struct: `_hasValue`, managed partial only | 2 |
| 3a | open | Class: private parameterless ctor, no `_hasValue` | 2 |
| 3b | open | Nested `{Stem}Case` case types | 1c |
| 3c | open | `Value` / `HasValue` / `TryGetValue` | 3, 3a, 3b |
| 3d | open | `[Union]` + `IUnion` via joined interface list | 3c |
| 4 | open | `ToUnmanaged` / `AsUnmanaged` empty guard | 3 |
| 4a | open | `ToManaged` constructs via case ctors + validates tag | **0, 0c** |
| 4b | open | Exception split: `InvalidOperationException` vs `InteropException` | 4, 4a |
| 5 | open | Flag-off snapshots byte-identical | **real checkout, not worktree** |
| 5a | open | Flag-on snapshot | 5 |
| 5b | open | net11/preview compile fixture | 3d |
| 5c | open | Collision cases: `Value`, casing-fold, `B`/`IsB` | 1a |
| 5d | open | `default(struct).ToUnmanaged()` throws | 4 |
| 5e | open | Class union cannot produce non-null empty | 3a |
| 5f | open | Invalid native tag throws | 4a |
| 5g | open | Default disposable `Dispose()` no-op | 3 |
| 5h | open | Managed-only `DataEnum` case | 3c |
| 6 | deferred | `Option` / `Result` — incl. `body_from_call` factory names | 5 green |

Note 0a is independent of the union work and of the `Variant` shape decision. It is a
one-line fix to a bug that affects ordinary unit-only enums, and it should not wait on
anything here.

Steps 1 and 2 no longer gate on Step 0 — name resolution and the builder flag touch nothing
the discriminant work touches. Only 4a does.

### Decided

**Discriminant lives on `Variant`** — item 0 unblocked. `VariantKind` becomes `Unit` /
`Tuple(TypeId)`, a pure payload descriptor. Rationale in `09b82d44`: the measured cost over the
alternative is four one-line match arms, and the same defect has already appeared
independently in three places.

**Defect 1's fix is not the one-line counter change described earlier.** `emit.rs` emits an
explicit discriminant as `(#expr) as isize` — a token stream evaluated at the *call site*, not
at macro-expansion time. The macro therefore cannot know that `A = 5` is `5`, and cannot resume
a numeric counter from it. `next_discriminant = disc + 1` is not implementable as written.

The fix is to carry the previous discriminant as a `TokenStream` and emit implicit variants as
`((#prev) + 1)`, keeping evaluation at the call site where the expression is const-evaluable.
Still small, but a different shape — and it means 0a is no longer a trivial one-liner that can
be waved through.

### Not doing

`[Union]` recognition, exhaustive matching and implicit conversion tests — compiler features.
Cached statics for payload-free `Value` cases — deferred optimisation, adds naming surface.
Multi-field and named variant support. Retiring `IsOk` / `AsOk`.

---

## Open items

1. **Step 0 shape** — `tag` on `Variant` vs. in `VariantKind::Tuple`. **Blocking.**
   Tracked in `09b82d44`; recommendation there is the field on `Variant`.
2. Null reaching the marshaller for a class-backed union — today's `NullReferenceException`,
   or a deliberate `ArgumentNullException` / `InteropException`?
3. Whether the proc macro currently sees explicit discriminants on payload variants. A read,
   not a change. Decides whether `09b82d44` is plumbing or real discriminant-assignment logic.

### Closed

- ~~`body_from_call`~~ — **not affected.** The pass `continue`s on anything that is not
  `TypePattern::Result`, so it never fires for a plain `DataEnum`. It also constructs through
  the factories (`return Ok(func())`, `return Panic`), not by mutation, so it needs no Step 4
  treatment. It *does* consume factory names, which makes it a **Step 6 naming consumer** —
  add it to the migration list when `Option` / `Result` land.
- ~~`builder.rs` setter shape~~ — confirmed, see Step 2.
- ~~`master.rs` routing~~ — confirmed, see Output routing.
- ~~Snapshot baseline blocks all template work~~ — retracted, see Prerequisites. Baseline is
  effectively green; the 29-of-33 was a Git LFS materialization failure.

---

## Rejected alternatives

| Option | Why rejected |
|---|---|
| `union` keyword declaration | Boxes every payload; forbids instance fields; always a struct |
| Managed ordinal remapped from native tag | Turns a copy into an N-way translation in both directions on every marshal |
| `_variant == 0` treated as a valid case for `default(T)` | Unsound: a well-formed struct union must have `default(T).Value == null` |
| Eager `_boxed` field for `Value` | Allocates in `ToManaged`, i.e. on every boundary crossing |
| Excluding enums without a zero discriminant | Arbitrary; `_hasValue` solves it for one bit |
| `unions: bool` in each output-pass config | Permits inconsistent state; superseded by projection-pass eligibility |
| Bare variant names for case types | Collide with existing factories (CS0102) |
| Unconditional PascalCase stems | Silently renames existing public members on non-colliding enums |
| CS8509 compile test for union recognition | Recognition is not the fragile part; the test would pass either way |
| Coupling this to full multi-field / named variant support | Independently shippable; that rewrite spans core, proc macros and every backend |
