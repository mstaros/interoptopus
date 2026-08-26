# C# 15 union projection for Rust enums

Status: **in progress.** § Todo/Remaining below is the single source for what is landed and what
is open; this line deliberately does not restate it. See `docs/csharp-unions-handoff.md` for
current state and traps; this file is the design record.

Scope: project every Rust `DataEnum` reaching the C# backend as a C# 15 custom union, subject to
the eligibility rule below. Native ABI unchanged. Not opt-in — the repository targets net11
everywhere and unions are the default enum projection (see Decided).

**Provenance — two sources, and they are not the same source.** Language claims here are read
from the unions feature specification at `MpsAgent/Unions/UnionSpecification.md`, a Microsoft
Learn snapshot of `dotnet/csharplang/proposals/unions.md` at `git_commit_id
f23bdbed3f5a9c5f5d78f7f2a0a9f0bc54ac58b4`, `ms.date 2026-06-02`. Compilation claims are verified
against the .NET 11 SDK **preview 7, August 2026**. The feature has shipped, so where that
snapshot leaves a question open the shipped implementation is authoritative — note that several
sections in it are headed `[Resolved]` and carry no resolution text, including both sections on
classes as union types. Do not read those as unresolved.

## Two layers, two rules

This plan governs the **generated layer** only. It is not a campaign to replace C# enums.

**Generated layer (this document).** Interoptopus projects Rust sum types faithfully. A
`DataEnum` goes through the union machinery unless it is **pure unit-only** — see the eligibility
rule below.

The original rule here was uniform: every `DataEnum`, *including unit-only ones* such as
`FfiObjectType`. Not because a union is better for a scalar choice — it is not — but because the
generator cannot tell a scalar choice from a payload alternative. That is domain knowledge it does
not have, so any eligibility rule based on variant shape would be a guess.

**That argument covers *domain* eligibility, not *mechanical* eligibility, and the distinction is
now settled.** Whether any variant carries a payload is not domain knowledge — it is
`VariantKind::Tuple` versus `Unit`, visible in the inventory. The domain half stands and still
governs everything else: the generator does not guess whether a payload-carrying enum is
"really" a scalar choice.

**Eligibility rule: a `DataEnum` with no payload-carrying variant does not receive union
machinery.** No case types, no `Value`, no `HasValue`, no `TryGetValue`. The rule is anti-bloat
and needs no population count to justify it: such an enum has no payload, so its case types are
empty, its `Value` is nothing and `TryGetValue` has nothing to get. `Issues.md` `79be256e` records
the rule, the measured cost (about 120 lines per enum: a struct, a discriminant field, an
`Unmanaged` mirror, a marshaller) and the population it applies to.

**Excluded enums keep their current representation.** Struct, `Unmanaged` mirror and marshaller
all stay; this rule only declines to *add* union machinery on top. Projecting them as plain C#
`enum`s instead is **option C in `79be256e` and is deferred** — it rests on a blittability claim
that is not yet verified, and separately on closed enums, which did not ship in C# 15.

**This document decides from the C# specification and the Rust inventory, and from nothing else.**
A downstream consumer may *motivate* a shape by demonstrating that it occurs in practice; it never
*constrains* what is emitted. One consumer's naming conventions, style rules or public-surface
policy are not inputs here. Earlier drafts cited a particular consumer's domain types as worked
examples and one of its style rules as a constraint; both were removed and the decision that
depended on the latter was re-derived from the language.

**Consumer layer (out of scope here).** The public API a consumer exposes is a
separate decision, made per type:

| Domain shape | C# form |
|---|---|
| Payload or state alternatives | union |
| Scalar choice | `enum` |
| Bit combinations | `[Flags]` enum |
| Product data | `record` / `struct` |

The test is whether the product type permits impossible states. A record carrying a target, a
referent, and two independent `IsX` booleans admits combinations that cannot occur — both flags
set, or neither with a referent present. A closed sum type makes those unrepresentable; that is a
union. A closed set of numeric values with no per-case payload is not: it stays a plain `enum`,
and bolting a `.SomethingCase` onto it would be a straight regression.

Where the generated internal representation of a unit-only enum becomes union-like, the
consumer translates at the boundary rather than propagating case types outward.

---

## 1. Representation: a manual `[Union]` type, not the `union` keyword

`[Union]` here is `System.Runtime.CompilerServices.Union` — the real attribute, not one of ours.
Writing the type by hand is a documented path, not a departure from the specification, which
names our exact case: *"You might need different behavior if you want to adapt an existing type,
create a class-based union, or use a custom storage strategy, or if you need interop support."*
An earlier draft of this section read "custom `[Union]`", which implied a bespoke attribute and
put the whole approach off-spec. It is not.

What *is* rejected is the **keyword**, and the specification describes the generated form in the
same terms this section arrived at independently: it *"is always a struct, always boxes
value-type cases, and always stores contents as `object?`"*. Three reasons, in order of weight:

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

**Scope: this applies per *enum*, not per variant.** A pure unit-only `DataEnum` is excluded from
union machinery entirely by the eligibility rule in §"Two layers, two rules" and never reaches
this step. But every variant of an enum that *is* projected as a union gets a case type,
**including its unit variants** — an empty case type is the solution to the unit-variant problem,
not a cost. Folding the unit variants of a mixed enum into one nested C# enum was considered and
dropped: it moves those variants out of the compiler-checked layer into a plain-enum switch, which
C# does not check for exhaustiveness. See `Issues.md` `79be256e`.

So 3b's scoping condition is a single check at the top of the pass — *does this enum have any
payload-carrying variant?* — not a filter applied to each variant.

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

**Step 0 is done** — `c928d53e`; `Issues.md` `09b82d44` is closed. It was the one hard
prerequisite: every downstream step assumes `_variant` is the native discriminant, and item 4a's
validated `ToManaged` switch would otherwise validate against the wrong tag set.

**Snapshot baseline.** Effectively green. `Issues.md` `ccb105a2` measures **30 passed, 1 failed,
2 ignored** of 33 on `b42399a4`; the single failure is `reference_project::interop`, awaiting
`cargo insta review` for an unrelated `AsSpan()`/`ToArray()` template change. It is open and
low-severity — one snapshot's bookkeeping, not a gate on template work.

An earlier version of that issue reported 29 of 33 failing and was cited in earlier drafts of
this plan as a hard blocker. **That was retracted** — it came from a Git LFS materialization
failure, not from the repository.

**LFS is gone, and with it two hazards this section used to carry.** Nine extensions were
LFS-tracked, three of them text (`*.json`, `*.snap`, `*.svg`) and none large — `global.json` is
63 bytes, the largest committed DLL was 30 KB. `.gitattributes` now classifies by text versus
binary instead. Consequences for this plan:

- **Transactions work.** `Issues.md` `1383b84b` is closed: the MCP commit pipeline built its
  candidate tree without the LFS clean filter, so the guard refused every transaction regardless
  of content. With nothing tracked there is nothing to trip on. Verified end to end by
  `2b1825f7`, a transaction that built a real candidate tree and integrated. Earlier revisions
  of this section told you to work in the real checkout and commit with plain `git`; that is no
  longer necessary. The underlying tooling defect is unfixed, so it returns if LFS tracking does.
- **`cargo insta review` is no longer destructive on a fresh clone.** Snapshots were pointer
  stubs without `git lfs pull`, and accepting against a stub overwrote the pointer with raw
  content. They are ordinary blobs now.

**Toolchain — in place.** Union output needs `<LangVersion>preview</LangVersion>` and a .NET 11
Preview 5+ SDK for `UnionAttribute` / `IUnion`. Step 2 landed both: 14 `.csproj` files retargeted
to `net11.0`, and `crates/backend_csharp/Directory.Build.props` sets `LangVersion=preview` for
everything beneath it. Since `8c70868d` the SDK is needed to run the tests at all, because
`define_plugin!` builds the reverse-interop plugins during the run. There is no opt-in flag, no
eligibility gate and no flag-off output to preserve — see Decided.

---


## Step 0 — Discriminant becomes an attribute of the variant

**Done — `c928d53e`, plus the earlier proc-macro fix. `Issues.md` `09b82d44` is closed.**

`VariantKind` carried the discriminant for `Unit` and nothing for `Tuple`, so backends
substituted the positional index for payload variants and the managed `_variant` disagreed with
the native tag for any enum mixing explicit discriminants with payloads.

Three defects, all fixed:

1. The proc macro never resumed the counter after an explicit discriminant, so
   `enum E { A = 5, B, C }` produced 5, 1, 2 instead of 5, 6, 7. It could not: the explicit
   value is emitted as `(#expr) as isize`, a token stream evaluated at the *call site*, so the
   macro never learns it is `5`. Fixed by carrying the previous discriminant as a `TokenStream`
   and emitting implicit variants as `((#prev) + 1)`.
2. Tuple variants discarded their discriminant entirely.
3. Three consumer sites substituted the positional index — `enum_variants.rs` and both
   `wire/mod.rs` serialize/deserialize paths, which re-derive independently of the model pass.

**Resolution.** `tag` is now a field on `Variant`, unconditional and independent of payload
shape; `VariantKind` is `Unit` / `Tuple(TypeId)`, a pure payload descriptor. Chosen over adding
a second element to `VariantKind::Tuple` because the measured cost was four one-line match arms
and the same defect had already appeared independently in three places.

Guarded by `EnumExplicitThenImplicit { A = 5, B, C }` and
`EnumExplicitPayload { A = 10, B(u32), C(Vec3f32), D = 20 }` in the reference project. The
second is the only enum giving a payload variant a real discriminant, and `D = 20` proves the
counter resumes across a payload variant.

The API hash changed; consumers must regenerate bindings.

---

## Step 1 — `union_names` model pass

**Done.** Shipped as `pass/model/common/types/union_names.rs`, sibling to `names.rs`. Earlier
drafts called it `union_projection` and placed it under `types/enums/`; both are stale. It owns
*naming only* — enablement no longer exists, since unions are the default.

`common` here means common to the rust and dotnet pipelines within `backend_csharp`, not
backend-neutral. `names.rs` already does C# casing there.

### What it produces

Resolved names live on `lang::types::kind::Variant` as `stem` and `case_type`, not in a side
table. Several output passes filter variants before emitting — `body` keeps only disposable
ones, `body_as_unmanaged` only payload-carrying ones — so a parallel vector indexed positionally
would misalign silently after any filter.

Every emitted member derives from `stem`: factory `{stem}`, check `Is{stem}`, accessor
`As{stem}`, field `_{stem}`, unmanaged helper `Unmanaged{stem}`, case type `{case_type}`.
`Variant::name` is the Rust spelling. It is **not** diagnostics-only: `wire/mod.rs` emitted it directly at six sites until item 1d, and `Issues.md` `4e9a17c3` is the record of that. Treat any claim that a field is unused as a claim to verify, not to repeat.

### Allocation policy

Preservation-biased: a variant keeps the name the generator emits today unless a fixed union
member makes that impossible. Three phases, and the ordering is load-bearing:

1. **Preserve.** Claim every stem that can keep its current name.
2. **Fallback.** Only then allocate `{stem}Variant`, `{stem}Variant2`, … for the rest.
3. **Case types.** New, so they move on collision rather than disturbing a working stem.

Without phase 1 preceding phase 2, a variant needing a fallback steals a name another variant
already emits: given `Value` and `ValueVariant`, single-pass allocation hands `ValueVariant` to
the first and displaces the second, breaking API that had no collision.

Stems are the Rust variant name **verbatim** — the templates emit it unmodified today, so
re-casing would rename members on enums with no collision at all. Templates must not re-sanitize
after this pass. Keyword escaping is deliberately out of scope; see `Issues.md` `7c8cb22e`.

**Open: the family is allocated unconditionally, with no unit/tuple distinction.** Three of the
six derived names — `_{stem}`, `Unmanaged{stem}`, `{case_type}` — exist only for payload-carrying
variants, so reserving them for a unit variant lets a collision on a never-emitted member move the
stem and rename `Is{stem}`/`As{stem}`. That cuts against this section's own policy rather than
with it. There is a real counter-argument from forward compatibility, and a reason to verify at
the emission sites before acting; both are recorded in `Issues.md` `79be256e`.

### Reserved names

`Value`, `HasValue`, `TryGetValue`, `Unmanaged`, `Marshaller`, `MarshallerMeta`, `ToUnmanaged`,
`AsUnmanaged`, `ToString`, `Dispose`, `ExceptionForVariant`, `_variant`, `_hasValue`, plus the
enclosing type's own name (CS0542).

`ToManaged` is absent on purpose: it is declared inside the nested `Unmanaged` and `Marshaller`
types and never shares a declaration space with an outer factory.

### Collision classes

`Foo`/`IsFoo`, `Foo`/`AsFoo` and `Foo`/`UnmanagedFoo` are **pre-existing broken output** — those
three names are already emitted today — so the pass fixes them rather than introducing risk.
Only `{stem}Case` is genuinely new. Twelve unit tests cover these plus the `Value`,
enclosing-name, `variant`/`_variant` and repeated-fallback cases.

### Consumers migrated

Nine sites read `v.stem`: `body`, `body_ctors`, `body_tostring`, `body_unmanaged`,
`body_unmanaged_variant`, `body_to_unmanaged`, `body_as_unmanaged`, `body_exception_for_variant`,
`definition`. Reference-project output is byte-identical; no snapshot moved.

**There is a tenth site, and it was missed** — `pass/output/common/wire/mod.rs` still emits
`Is{name}`, `As{name}()` and the factory from the raw Rust `variant.name`, at six places. It is
not a rename: `WireCodeGen` holds only `&RsTypes` and cannot reach the C# model at all, so it
also skips the `sanitize_rust_name` that every `names.rs` path applies to type names.

Latent today because `stem == name` throughout the reference project, and it becomes a compile
error in the generated file once a stem moves — `body.cs` emits `Is{stem}` while wire emits
`Is{name}`. Items 3b and 3c both add collision surface, so this fires as Step 3 lands. Tracked
as `Issues.md` `4e9a17c3`; item 1d.


### Ordering constraints

Two, both found by breaking them:

- Write to **`type_kinds`**, not `type_all`. `type_all` rebuilds its `Type` values from
  `type_kinds`, so writing there populates a copy that is later overwritten.
- Run **before `type_all.process`** — immediately after `type_names.process` in both pipelines.

And one shape constraint: a `DataEnum` arrives by three routes, not one. `Option` and `Result`
carry theirs inside `TypePattern`, so matching only `TypeKind::DataEnum` leaves `Ok`/`Err`
unresolved and emits `_` for every payload field. Use `data_enum` / `data_enum_mut`.

`PostModelPass` was deliberately not extended — it is a narrow extension view, not a mirror of
every model pass.

---

## Step 2 — Target framework (replaces the builder flag)

**Done — `9d664613`.** This step was originally an opt-in `RustLibraryBuilder::unions(bool)`;
that is dropped. See Decided: the repository targets net11 everywhere and unions are the default
enum projection, so there is no flag, no eligibility gate, and no flag-off output to preserve.

What landed:

- **14 `.csproj` files** moved `net10.0` → `net11.0`.
- **`crates/backend_csharp/Directory.Build.props`** (new) sets
  `<LangVersion>preview</LangVersion>` for everything beneath it. Chosen over 14 per-project
  elements so that `InteropSpike.csproj` — already `net11.0`, so it would not have matched a
  retarget pattern — is covered, new projects inherit it, and there is one place to delete at GA.
- **Justfile** — `_bdp_ref` and `_bdp_p` copied from `bin/Release/net10.0/`; now `net11.0`.
- **`crates/backend_csharp/src/rt/dynamic.rs`** — see below.
- **11 plugin DLLs** rebuilt against net11.

`crates/backend_csharp/global.json` pins no SDK — it only selects
`Microsoft.Testing.Platform` as the test runner — so nothing there blocked the retarget.

### The runtime config, which this plan originally missed

`rt/dynamic.rs` holds `DEFAULT_RUNTIME_CONFIG`, a hard-coded hostfxr runtime config pinned at
`tfm: net10.0` and `framework.version: 10.0.0`. It is written to a temp file and passed to
`initialize_for_runtime_config`, so it decides which runtime the plugin host boots.

The original Step 2 inventory listed the build configuration — `.csproj` files and the Justfile —
and never asked what pins the framework at *runtime*. Retargeting without it left all 12
plugin-load tests failing with `SymbolNotFound` for eleven different symbols, which reads like a
codegen fault rather than an assembly that never loaded. Diagnosis required tracing
`symbol_not_found` back to `loader(#symbol)` returning null in `proc_macros_impl/src/plugin/emit.rs`.

Setting the version to a plain `11.0.0` then produced a second, sharper failure: *"It was not
possible to find a compatible framework version"*, listing `11.0.0-preview.7.26381.103` among the
installed frameworks. **hostfxr will not roll forward from a release request to a pre-release
runtime.** The version is therefore `11.0.0-preview.1` — requesting a pre-release enables
pre-release resolution, and `rollForward: LatestMajor` picks the newest installed 11.x. It stays
correct after GA, since a release version outranks any pre-release.

Generalising: a framework retarget in this repo has **three** classes of pin, not two — build
configuration, build tooling, and the runtime config the Rust host boots. Only the third has
teeth at test time.

### Toolchain

Requires a .NET 11 Preview 5+ SDK for `UnionAttribute` / `IUnion`; verified against
`11.0.100-preview.7.26381.103`. Until C# 15 goes GA (expected November 2026) `LangVersion` must
stay `preview`, and the union spec still carries open questions that could move the emitted shape
before then.

**The preview gate is verified — measured 2026-08-26, ahead of Step 3.** A scratch project on
preview 7, built twice against `LangVersion=preview` and `LangVersion=13`, with a manual `[Union]`
struct carrying two nested `readonly record struct` case types, one public single-parameter
constructor each, and a public `object? Value`:

| Construct | `preview` | `13` |
|---|---|---|
| `[Union]` declaration, nested case types, constructors, `Value` | compiles | **compiles** |
| Explicit `new Shape(new Shape.CircleCase(1.0))` | compiles | **compiles** |
| Implicit `Shape s = new Shape.CircleCase(1.0)` | compiles | CS8652 |
| `switch` over case types, no default arm | compiles, no CS8509 | CS8652 |

CS8652 names the feature `unions` explicitly, so the gate is real and `LangVersion=preview` is
load-bearing — but only for **use**, not for **declaration**.

**Open, and it may narrow this section considerably.** The declaration shape and explicit
construction are ordinary C# 13. Generated bindings construct through factories and, under item
4a, through a validated switch of constructor calls — none of which is a conversion or a union
pattern match. So it is possible that generated output needs no preview at all and the
requirement falls entirely on consumer code that pattern matches. **Not yet established**: a
second probe containing only emitted shapes is the way to settle it, and until then this section
keeps the blanket requirement.

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

- No `_hasValue` field. `8c70868d` added it to struct-backed enums only, which is correct.
- `HasValue => true` (constant).
- `Value` is never null.
- A **private parameterless constructor** replaces today's implicit public one, so
  `new EnumX()` can no longer produce a bogus variant-zero instance. Factories, case
  constructors and `Unmanaged.ToManaged()` construct from inside the type and are unaffected.

**Breaking change, unconditionally.** There is no flag, so external `new EnumX()` stops
compiling for every class-backed enum the moment 3a lands. Changelog entry required. Consumers
that never construct generated types directly are unaffected, but that is a property of the
consumer, not something this projection guarantees — verify it when regenerating.

**OPEN:** what happens when `null` reaches marshal-out. Measured: a class-backed union stored as
a composite field gets an unguarded `.AsUnmanaged()`, so the NRE fires inside the enclosing
composite's conversion, not in a marshaller (`nullable.rs:35`, `body_as_unmanaged.rs:47`). The
decision is `InvalidOperationException` vs `ArgumentNullException` vs joining the existing
`?? default` policy — the last silently fabricates discriminant 0 and must not be taken by
default. Open item 1.

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

These switch arms are keyed on `_variant`, which is only the native discriminant because Step 0
landed (`c928d53e`). Before that they would have validated against the wrong tag set.

**`Result` and `Option` go through this path too**, so two constraints that earlier drafts
parked in Step 6 are requirements here (item 4c): `default(ResultX)` must be **empty**, not
`Ok`, and `default(OptionX)` must be distinct from `NoneCase`. `8c70868d` already emits
`_hasValue` on the `Result` carriers, so the storage exists; what is missing is the reads.
One observable consequence: `AsOk()` on a default `Result` throws the empty-state exception,
where today any failure surfaces as `EnumException<E>`. Consumers translate that at a single
boundary, so the change is visible to them — flag it in the changelog alongside the class-ctor
break.

### Exceptions

A default struct union is a legal C# state, not corruption. `InteropException` in this
codebase means *"severe error, should never happen"*.

| Failure | Exception |
|---|---|
| empty `AsX()` / marshal-out | `InvalidOperationException` |
| unknown native discriminant | `InteropException` |

`ExceptionForVariant()` returns the empty-state exception so existing
`throw ExceptionForVariant()` call sites stay coherent.

This is **decided**, not open — item 4b is implementation only.

---

## Step 5 — Tests

**Not tested:** `[Union]` recognition and exhaustive matching. Those are compiler features, and
a test would pass either way. *Emitting* whatever a custom `[Union]` needs is still ours — see
Open item 2 on case conversions.

**The compile gate is the real consumer path, not a fixture.** Step 2 retargeted `Bindings`,
`Tests` and the plugin projects to net11 with `LangVersion=preview`, so they compile union
output directly. That supersedes the bespoke net11 fixture earlier drafts specified, and it is
the only thing that verifies `LangVersion=preview` is doing anything — the native API guard
cannot, because the projection leaves the ABI and hash unchanged.

Since `8c70868d`, the plugin projects are also built by `define_plugin!` during `cargo test`
rather than by a separate `just build-dotnet-plugins` step, so that compile gate runs on every
test invocation. `_hasValue` currently trips **CS0169: never used** on eight generated types;
that clears with item 3c and is a useful marker until it does.

There is no byte-identical regression gate. Snapshots move once, when the projection lands, and
are reviewed rather than diffed to zero.

**Tested — our behaviour:**

- one focused union snapshot
- variant named `Value`
- ~~casing-fold collision~~ — stale, no such class: `union_names` allocates case-sensitively, and there is no case-insensitive comparison anywhere in `backend_csharp`
- `B` / `BCase`, and cross-family `B` / `IsB` — both managed-side only
- **a stem-moving collision reachable inside `Wire<E>`** — the cases above never reach the wire emitters, which is exactly why `4e9a17c3` stayed invisible; `B`/`BCase` in particular moves only `case_type`, leaving both stems intact. Covered by `tests/output/wire/collision.rs`
- `default(struct E).ToUnmanaged()` throws
- class-backed union cannot produce a non-null empty instance
- invalid native tag throws
- default disposable struct `Dispose()` is a no-op
- one managed-only `DataEnum` (`body.rs` supports `DataEnum`s with no `Unmanaged` form)
- **`default(ResultX)` is empty, never `Ok`** — a Step 3/4 requirement, not a deferred one,
  because `Result` is projected in this pass
- **`default(OptionX)` is distinct from `NoneCase`** — same
- **`AsOk()` on a default `Result`** throws the empty-state exception, not `EnumException<E>`.
  Consumers catch the latter at a single translation boundary, so the change is observable
- existing Rust round trip unchanged

---

## Step 6 — `Option` / `Result` leftovers

**Not an exclusion.** `Option` and `Result` carry a `DataEnum`, so they get the union projection
along with everything else — the machinery in Steps 3 and 4 is written generically and does not
discriminate by type. Naming is *already* done for them: `union_names` resolves all three
carriers, and `result_and_option_variants_resolve_unchanged` guards it.

Earlier drafts described this step as "deferred until the plain case is proven", which read as
"`Result` might not become a union". It will. What is deferred is the `Result`-specific tidying
that has nothing to do with whether it is a union:

- `Result` implements `IResult<T,E>` with `AsOk()` / `AsErr()` and unit-side methods. Case types
  must coexist with that interface, not replace it.
- `body_from_call` constructs `Result` through the factories (`return Ok(func())`,
  `return Panic`). It never fires for a plain `DataEnum` and needs no Step 4 treatment, but it
  does consume factory names, so it is a naming consumer to re-check here.
- `default(ResultX)` must be **empty**, not `Ok`. `default(OptionX)` must be distinct from
  `NoneCase`.

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

Execution state. Rationale lives in the step sections above; this tracks only what is done.

### Done

| # | Item | Landed |
|---|---|---|
| 0a | Proc macro resumes implicit discriminants from the previous value | `09b82d44` defect 1 |
| 0d | `EnumExplicitThenImplicit { A = 5, B, C }` — regression test | same commit |
| 0 | Discriminant moved onto `Variant.tag`; `VariantKind` is payload-only | `c928d53e` |
| 0b | Proc macro emits `tag` for every variant, `Unit` carries none | `c928d53e` |
| 0c | 3 index-as-tag sites: `enum_variants:54`, `wire:318`, `wire:352` | `c928d53e` |
| 0e | `EnumExplicitPayload { A = 10, B(u32), C(Vec3f32), D = 20 }` → 10, 11, 12, 20 | `c928d53e` |
| R | net11 retarget, `LangVersion=preview`, `rt/dynamic.rs`, plugin DLLs | `9d664613` |
| 1 | `union_names` model pass + preserve/fallback/case-type allocator, 12 unit tests | *(union_names commit)* |
| 1a | Reserved-name set incl. enclosing type name (CS0542) | same |
| 1b | Stem = currently-emitted name verbatim, not re-cased | same |
| 1c | All **nine** name-deriving sites emit from `v.stem`; output byte-identical | `2bdbf054` |
| 3 | Struct-backed data enums emit `_hasValue` in the managed partial | `8c70868d` |

`Issues.md` `09b82d44` is closed; `2a6da76a`, `ccb105a2`, `1383b84b` and `7c8cb22e` remain open.

Item 3 emits the field but nothing reads it yet, so every generated carrier currently warns
**CS0169: the field `_hasValue` is never used** — eight types across five reference plugins at
the time of writing. That is expected and clears with item 3c, which is what consumes it.

Earlier drafts named item 1 `union_projection` and gave it an eligibility gate. Both are stale:
the pass is `union_names`, it owns naming only, and there is no gate — see Decided and Step 1.

### Remaining

Items 0–1c, R (the .NET 11 retarget — `LangVersion=preview`, `rt/dynamic.rs`, plugin DLLs;
`9d664613`, in the Done table above), 1d, **3a, 3b, 3c and 3f** are done. **3d and 3e are the open
front**: 3d's gate (3c) and 3e's gate (3b) are both satisfied. **4** has no gate and can run in
parallel.

3a and 3b were independent and landed separately — 3a is representation (a private parameterless
constructor on class-backed enums), 3b is emission (nested case types). Both are mutation-proven:
the guard was inverted and the test observed to fail before being restored byte-identical.

1d was previously named here as the first thing to fix; it landed in `f5057d4b`. It gated 3b
because 3b introduces `{case_type}`, a new collision class, and every collision it resolves moves
a stem — which was a place `wire` and `body.cs` disagreed until 1d closed it.

**Unresolved in this line:** the claim that item "3" is done sits alongside 3a–3f listed open
below. Either "3" is a superseded coarse item that 3a–3f replaced, or the claim is wrong. It was
not determinable from this document and has not been guessed at.

| # | Status | Item | Gate |
|---|---|---|---|
| 1d | **done** `f5057d4b` | ~~**`wire/mod.rs` bypasses the C# model.**~~ Six sites emit `v.name`; `cs_type_name` skips `sanitize_rust_name`. `Issues.md` `4e9a17c3` | — |
| 3a | **done** `f630e225` | ~~Class: private parameterless ctor, no `_hasValue`~~ Declaring any constructor removes the implicit public one, so a single `private E() { }` is the whole change; nested types may still reach it, so factories and `Unmanaged.ToManaged()` are unaffected. The `_hasValue` half was already correct — `8c70868d` added that field to struct-backed enums only. Breaking change, changelog entry added | — |
| 3b | **done** `4a19b0e3`, tests `9d6905bd` | ~~Nested `{case_type}` case types~~ New pass `body_case_types.rs`, scoped per enum by `variants.iter().any(\|v\| v.can_carry_payload)`. **Corrected after this row was first written:** it read `v.ty.is_some()`, which asks whether a variant *does* carry a payload rather than whether it *can*. `fallback.rs::resolve_payload` erases a `()` payload to `None`, so `Result<(), ()>` read as entirely payloadless and lost its projection while `Result<u32, Error>` kept it — the same shape decided by a type argument. The predicate is now the model-layer field `Variant::can_carry_payload`, set from `VariantKind::Tuple` in `enum_variants.rs` and from the payload-carrying position in `fallback.rs`; five sites read it. A `DataEnum` with no payload-**capable** variant is skipped entirely, while a union-projected enum gets a case type for **every** variant, unit ones included. Guard test `enum_union_members::eligibility_asks_can_carry_not_does_carry` is the only fixture separating the two predicates — reverting passes the whole rest of the suite. Names came from `Variant::case_type`, already allocated and collision-resolved by `union_names`. Remains the precondition for 3e; see Open items #2 | 1d ✔ |
| 3c | **done** `bdd13b53` | ~~`Value` / `HasValue` / `TryGetValue` — consumes `_hasValue`, clears CS0169~~ Same scope as 3b, through `can_carry_payload`. Struct-backed: `HasValue => _hasValue`, `Value` returns null when it is false. Class-backed: `HasValue => true`, no `_hasValue` field, no `_hasValue &&` in `TryGetValue`. `Value` boxes a `readonly record struct` per access — value-stable, not reference-stable. Leaves a deliberate stopgap `_managed._hasValue = true;` in `ToManaged` for 4a to delete | 3a ✔, 3b ✔ |
| 3d | open | `[Union]` **only**. The attribute's full name is `System.Runtime.CompilerServices.UnionAttribute`, confirmed against a net11 assembly that builds (`MpsAgent/Unions`, `UnionReflection.HasUnionAttribute` matches on that string). ~~+ `IUnion` via joined interface list~~ — **`IUnion` is a separate decision and is not part of 3d:** the specification leaves its namespace unspecified, states the compiler will not synthesize it, and records `IUnion<TUnion>` as removed. Emit neither without a verified reference. **Hard constraint on 3c's output:** `Value` must remain a *public instance* property. Reflection consumers locate it with `GetProperty("Value", Public \| Instance)`, so an explicit `object? IUnion.Value => …` implementation would be invisible to them | 3c ✔ |
| 3e | open, **shrunk** | ~~decide whether the compiler synthesises it~~ — settled: it does, via the generated constructor. Reduces to "constructors are public and single-parameter"; nothing emitted for the conversion | 3b ✔ |
| 3f | **done** `4a19b0e3` | ~~**Case-type accessibility — `public`**~~ Emitted `public` at the site rather than inherited, since a nested type defaults to `private` — unusable, because the case type could not then be named outside the union — and `internal` fails the same way across an assembly boundary. Landed with 3b as planned; asserted by `enum_case_types::case_types_are_public` | 3b ✔ |
| 4 | open | `ToUnmanaged` / `AsUnmanaged` empty guard | — |
| 4a | open | `ToManaged` constructs via case ctors + validates tag | 3c ✔ |
| 4b | open | Exception split (decided in Step 4; implementation only) | 4, 4a, **Open items #1** |
| 4c | open | `default(ResultX)` empty not `Ok`; `default(OptionX)` ≠ `NoneCase`. **A soundness obligation, not a preference** — specification § Well-formedness, *Soundness*: `Value` always evaluates to null or to a value of a case type, **expressly including the default value of the union type**. (An earlier version of this row cited *"for struct unions, `default` produces a `Value` of null"*. That rule sits in the specification's `[Obsolete]` section and is marked removed; the obligation survives under *Soundness*.) The side state is **forced, not chosen**: `Ok` is tag 0 and `default` is all-zero, so given tag preservation — settled, and load-bearing because it keeps the crossing a memcpy rather than an N-way translation — the discriminant cannot distinguish them, and `_hasValue` or equivalent is the only remedy. Consequence recorded nowhere else: a `Value` that can be null makes a consumer's otherwise-exhaustive `switch` warn on unhandled null | 3c ✔, 4a |
| 4d | partial | Wire has **two** throw sites, not one. Deserializer's unknown native tag → `InteropException`: **done**. Serializer's empty-state `else` should call `ExceptionForVariant()` instead of its own "Unknown variant": **open**, needs 4b's helper | 1d ✔, 4b |
| 5 | open | Snapshots move once; consumer projects compile the output | 3d, 4b |
| 5c | partial | Collision cases: `Value`, `B`/`IsB`. ~~casing-fold~~ — no such class, `union_names` allocates case-sensitively. A Wire-reachable stem-moving collision is covered by `tests/output/wire/collision.rs`; the managed-side cases remain | 1d ✔ |
| 5d | open | `default(struct).ToUnmanaged()` throws | 4 |
| 5e | open | Class union cannot produce non-null empty | 3a ✔ |
| 5f | open | Invalid native tag throws | 4a |
| 5g | open | Default disposable `Dispose()` no-op | — |
| 5h | open | Managed-only `DataEnum` case | 3c ✔ |
| 5i | open | `default(ResultX)`/`default(OptionX)` tests; `AsOk()` on default | 4c |
| 6 | open | `Result` leftovers: `IResult<T,E>` coexistence, `body_from_call` factory names | 5 green |

**Two lists number separately, and the gate column names which.** `1` in the Done table above is
the `union_names` model pass, and it is done. `Open items #1` is the class-union null-at-marshal-out
question, whose three positions (a)/(b)/(c) are unmeasured in two of three — that is what gates 4b,
not the Done-table item.

Item 1d gates 3b because 3b introduces `{case_type}`, a new collision class — every collision it
resolves moves a stem, and every moved stem is a place `wire` and `body.cs` disagree. 5c is gated
on 1d rather than the reverse because a colliding variant is exactly the test that exposes it.

Item 6 is **not** an exclusion — `Option` and `Result` are projected in this pass, and `8c70868d`
already emits `_hasValue` on `Result` carriers (`ResultVoidError`, `ResultVec3f32Error`,
`ResultUintDotnetException`) alongside plain enums. Only the `Result`-specific tidying is
deferred; see Step 6. `body_from_call` is a naming consumer and moves here from the Closed list.

### Decided

**Target net11 everywhere; unions are the default, not a flag.** This repository is a fork with
a single known consumer, so there are no net10 consumers to protect. Dropped as a result: item 2
(`RustLibraryBuilder::unions(bool)`), the projection pass's eligibility gate, item 5's
byte-identical flag-off regression gate, and item 5b's separate net11 compile fixture. The
retargeted `Bindings`, `Tests` and plugin projects compile the union output directly, which is a
better check than a bespoke fixture because it is the real consumer path.

The cost: this is unmergeable upstream, since interoptopus proper cannot require a preview
compiler. Accepted deliberately. Reinstating the flag is the price of that option if it is ever
wanted back.

**Discriminant lives on `Variant`.** `VariantKind` is `Unit` / `Tuple(TypeId)`, a pure payload
descriptor. The measured cost over the alternative was four one-line match arms, and the same
defect had already appeared independently in three places. Landed in `c928d53e`.

**Defect 1's fix was not a one-line counter change.** `emit.rs` emits an explicit discriminant as
`(#expr) as isize` — a token stream evaluated at the *call site*, not at macro-expansion time —
so the macro never learns that `A = 5` is `5` and cannot resume a numeric counter from it. The
fix carries the previous discriminant as a `TokenStream` and emits implicit variants as
`((#prev) + 1)`, keeping evaluation where the expression is const-evaluable.

### Not doing

`[Union]` recognition, exhaustive matching and implicit conversion tests — compiler features.
Cached statics for payload-free `Value` cases — deferred optimisation, adds naming surface.
Multi-field and named variant support. Retiring `IsOk` / `AsOk`.

---

## Open items

1. **Null at marshal-out for a class-backed union.**

   **Direction.** Marshal-out only — `ToUnmanaged()` / `AsUnmanaged()`, and the same call emitted
   on an enclosing composite's field. `Unmanaged.ToManaged()` constructs from native bytes and can
   never receive a managed null, so it is out of scope.

   **Position.** Three, and they are not one question: (a) the union instance passed as an
   argument; (b) a class-backed union stored as a field of a composite; (c) a null element in a
   collection of unions. `GetUninitializedObject` does **not** bound these — it is emitted only at
   `wire/mod.rs:104` and `:264`, in the wire deserializer, a different path from marshal-out. It
   supports test 5e; it says nothing here.

   **Measured, not asserted.** Position (b) NREs today, and it does so *before* any marshaller.
   `nullable.rs:35` defines nullability as `TypeKind::Delegate(d) if d.kind == DelegateKind::Class`
   — class delegates only — and `body_as_unmanaged.rs:47` / `body_to_unmanaged.rs:48` emit the
   `?... ?? default` form only when that holds. A class-backed union field therefore gets a bare
   `.AsUnmanaged()`, and the dereference happens inside the enclosing composite's conversion, not
   in a marshaller. Positions (a) and (c) are **unmeasured**, and reproducing them is cheap — one composite
   carrying a class-backed union by value, one carrying a collection of them, through the
   existing corpus. **Do it before 4b**: a different failure mode in either changes what 4b
   implements, and 4b is the wrong place to discover it.

   **The choice is three-way, not two.** `InvalidOperationException`, for consistency with the
   Step 4 row for the struct empty state; `ArgumentNullException`, the conventional .NET answer
   for a null argument at a public boundary; or joining the existing `?? default` policy and not
   throwing at all.

   The third is the trap. `nullable.rs` promises "reference type / class" in its doc-comment and
   implements "class delegate"; after 3a class unions *are* reference types, so the gap invites a
   one-line "fix". Taking it makes a null union emit `?.AsUnmanaged() ?? default`, producing a
   zeroed `Unmanaged` — discriminant 0, a fabricated variant crossing FFI — which is exactly what
   3a's private constructor exists to prevent. **Silent default is worse than the NRE.** However
   this item is decided, class-backed unions must be excluded from that path explicitly. Filed as
   `Issues.md` `b4e07f12`, which is a **present** defect independent of this plan: `nullable.rs`
   already mis-classifies class-backed enums and class-backed composites, both of which exist
   today. 3a widens that set; it does not create it.

   **`InteropException` is excluded**, with its reason: it means *"severe error, should never
   happen"*, and a consumer passing null is an ordinary mistake, not corruption.

   **Gates 4b, not 3a.** 3a is the private parameterless constructor plus dropping `_hasValue`;
   neither depends on which exception is thrown. The Remaining table already gives 3a's gate as
   `—`; this item previously contradicted it and was wrong.

   **No test row covers this yet.** 5e ("class union cannot produce non-null empty") concerns a
   non-null invalid instance and does not exercise null at marshal-out. A row asserting "null
   union at marshal-out throws the decided exception" is missing; it is not added here because
   numbering new rows is not this item's call.
2. ~~**Case→enum conversion.**~~ **Closed — the compiler provides it, and this is now confirmed by
   compilation rather than by reading alone.** The spec says *"An implicit union conversion exists
   from each case type to the union type"*, and it *"works by calling the corresponding generated
   constructor"*. Measured 2026-08-26 on preview 7: `public static Shape FromCase() => new
   Shape.CircleCase(1.0);` — a case-type expression where a union is expected, no cast — compiles
   under `LangVersion=preview` and fails under `13` with CS8652 naming the feature `unions`. A
   `switch` over the case types with **no default arm** compiled with no CS8509, so exhaustiveness
   works as the projection assumes. See Step 2 §Toolchain for the full result table. Two sanctioned shapes, and we choose per type:
   the **basic union pattern** (a public constructor per case type, single by-value or `in`
   parameter, plus a public `object?` `Value`), or a **union member provider** (a nested
   `IUnionMembers` declaring static `Create` per case type) for types needing a private
   constructor or factory creation — the spec names `record class` unions.

   So item 3e collapses from an emission family to a constructor-shape requirement. Nothing is
   emitted for the conversion itself; what must be right is that the constructors are public and
   take one parameter each.

   **This makes 3b a precondition for 3e, not merely prior to it.** *"If more than one case type
   is equally applicable to the source value, the union conversion is ambiguous, and the compiler
   reports an error."* Rust enums routinely carry the same payload twice — `A(u32)`, `B(u32)` —
   and two constructors both taking `uint` are ambiguous. The distinct nested `{case_type}`
   wrappers are exactly what makes them resolvable.
3. ~~**Case-type accessibility.**~~ **Closed — the language forces it.** Nested `{case_type}` types
   have no stated accessibility, and in C# a nested type defaults to **`private`**. Inheriting that
   default does not give a suboptimal accessibility; it gives an unusable one — the case type
   cannot be named outside the union, so pattern matching cannot mention it, and exhaustiveness
   checking goes with it. That is the projection's main consumer-facing gain.

   `internal` fails the same way one scope out: consumers pattern-match across an assembly
   boundary. And under `IUnionMembers` the static `Create` signatures reference the case types, so
   they must be at least as accessible as that interface.

   So: **`public`**, stated explicitly at the emission site rather than inherited. Settle it during
   3b, when the case types are first emitted.

### Closed

- ~~**Step 0 shape**~~ — decided and landed: `tag` is a field on `Variant`. `c928d53e`.
- ~~Whether the proc macro sees explicit discriminants on payload variants~~ — answered by
  item 0e: `EnumExplicitPayload { A = 10, B(u32), C(Vec3f32), D = 20 }` yields 10, 11, 12, 20,
  so the counter resumes across a payload variant. Real logic, not plumbing.
- ~~`body_from_call`~~ — needs no Step 4 treatment (it constructs through factories, never
  mutates) but it *does* consume factory names, so it is a naming consumer. Moved to item 6.
- ~~`builder.rs` setter shape~~ — confirmed, see Step 2.
- ~~`master.rs` routing~~ — confirmed, see Output routing.
- ~~Snapshot baseline blocks all template work~~ — retracted, see Prerequisites. Baseline is
  effectively green; the 29-of-33 was a Git LFS materialization failure.
- ~~`Option`/`Result` are excluded from the first projection~~ — retracted. They carry a
  `DataEnum` and are projected with everything else; `8c70868d` demonstrates it by emitting
  `_hasValue` on the `Result` carriers. Only `Result`-specific tidying is deferred; see Step 6.

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
