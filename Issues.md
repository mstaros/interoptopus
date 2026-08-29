# Issues

## C# generation must reject missing Vec and Utf8String builtin helpers explicitly

```issue
id: 2a6da76a
kind: bug
severity: high
status: closed
```

### Symptom

The C# backend had two different missing-builtin failure modes, neither deliberate:

- A collected `ffi::Vec<T>` without matching create and destroy helpers was silently skipped by the Vec emitter. Other generated types still referenced the projected C# Vec type, so `RustLibrary::process()` succeeded with dangling references.
- A collected `ffi::String` without all three helpers reached the Utf8String template without its helper variables. Generation failed, but only because Tera happened to require missing context.

### Measurements — 2026-08-29

| Experiment | Generation | Observable gate |
| --- | --- | --- |
| Remove `builtins_vec!(ffi::String)` | Succeeds and writes a snapshot containing four `VecUtf8String` uses and no declaration | With the snapshot accepted, only `reference_project::csharp_suite` fails, at C# compile with `CS0246` |
| Reach that Vec through `Layer1<T>.maybe_2` at `T = ffi::String` | The concrete Vec is collected despite appearing in no direct function signature | Confirms the generic-composite route is visible to the C# model |
| Remove `builtins_string!()` | Fails before writing a snapshot | Both `reference_project::interop` and `reference_project::csharp_suite` stop in generation because the template lacks helper context |

The string result is not evidence of validation: its fast failure was an incidental property of the template.

### Revised scope

This is a C# backend generation defect, not a core `RustInventory::validate()` rule. Core validation is shared by the C, Python, and C# backends, and there is no measurement showing the other backends require these C# helper-backed wrapper types.

Diagnostics name the missing projected type and state that it may have been collected transitively through a field or enum payload. Exact field-level reverse provenance is intentionally out of scope: one projected type can have several use sites, and the current model has no complete reverse-origin graph.

### Resolution

- The C# Vec output pass now returns an ordinary generation error when matching create and destroy helpers are absent instead of continuing and omitting the declaration.
- The C# Utf8String output pass now returns an explicit missing-`builtins_string!()` error before template rendering instead of exposing a Tera context failure.
- Both errors name the projected C# type or element name and point to the registration macro.
- Core `RustInventory::validate()` remains unchanged.

### Regression coverage

Backend tests cover all three missing-helper routes:

- `ffi::Vec<T>` collected through a concrete `Layer1<T>` generic-composite instantiation;
- `ffi::Vec<T>` collected through an enum payload;
- `ffi::String` producing the explicit diagnostic rather than the old template error.

Verification on 2026-08-29: the focused regression set passed 3/3, cargo diagnostics reported zero errors and zero warnings, and the complete `interoptopus_csharp` package run passed 89 tests with zero failures and six existing ignores. Its embedded C# suite passed 222/222 tests.

## reference_project snapshot not re-accepted after the AsSpan()/ToArray() template change

```issue
id: ccb105a2
kind: bug
severity: low
status: closed
```

### Symptom

`cargo test -p interoptopus_csharp` fails one test: `reference_project::interop`. Measured on `master` at `b42399a4`: **30 passed, 1 failed, 2 ignored** of 33. The two ignored are `backend_plugins::memory::load_plugin{,_async}`, marked flaky in-source.

The diff contains only the `AsSpan()`/`ToArray()` addition from `templates/rust/pattern/vec/fast.cs`, plus its doc-comment change on the blittable `byte` vector. The generated inventory hash is unchanged at `0x6e82b3767f6b4dd`, confirming reference-project output did not move.

### Action

Run `cargo insta review` and accept `reference_project::interop`. No other snapshot needs re-accepting. Accept in the real checkout, not a worktree — see below.

### Correction to the previous version of this issue

This issue previously reported 29 of 33 tests failing, named `output::types::enum_basic::basic`, `output::patterns::result::basic`, `output::patterns::slice::non_blittable`, `output::services::basic::basic`, `reference_plugins::service::load_plugin_service_basic` and `backend_plugins::exceptions::build_plugin` among the failures, and attributed the drift to commit `b42399a4` changing `reference_project/src/services/asynk/wire.rs`. All six of those tests pass. The attribution was wrong on its own terms: `test_output!` in `tests/common/mod.rs` builds a fresh `RustInventory` per test from only the items that test registers, with `emit_version: false`, so a reference-project change cannot reach `output::*` at all — `enum_basic` registers one local `Color` enum. `b42399a4`'s `wire.rs` output was already present in the accepted snapshot as `service_async_wire_wire_passthrough(... WireOfHashMapStringString x ...)`.

The 29 was real but came from a Git LFS failure, not from this repository. `.gitattributes` tracks `*.snap` and `*.dll` in LFS. In a checkout where LFS content is not materialized — a fresh clone without `git lfs pull`, a CI checkout without LFS, or an MCP transaction worktree — every `.snap` is a three-line pointer stub and every prebuilt `_plugins/*.dll` is too. That fails 17 `insta` tests against empty baselines and 12 plugin-loading tests: 29 exactly, with `output::output` and `model::service_rval_result::result_types_have_distinct_names` passing and the two flaky memory tests ignored. `insta` reports `A snapshot uses a legacy snapshot format` when it parses a pointer. "`OptionLeaf` and `OptionMiddle` appearing as wholly new types" is what an empty baseline produces: everything is new.

Filed upstream as `10b7b672` in `rust-mcp-transform`.

### Hazard

The previous recommended action — `cargo insta review` — is destructive in that state. Accepting against stub baselines overwrites every LFS pointer with raw generated content, silently un-LFS-ing the snapshots. Before measuring this suite, confirm `crates/backend_csharp/tests/reference_project/snapshots/r#mod__reference_project__interop.snap` is ~847 KB and not three lines.

### Resolution — closed, the test passes

Measured 2026-08-26 on `master` at `318256c7`, in the real checkout, whole workspace:

```
cargo nextest run
Summary [34.862s] 174 tests run: 174 passed, 3 skipped
    PASS [1.431s] (134/174) interoptopus_csharp::mod reference_project::interop
```

Run twice (39.1s, 34.9s), identical result. `reference_project::interop` is neither failing nor
skipped — it passes. The reference snapshot measured 858,865 bytes, so the stub condition this
issue's Hazard warns about did not apply either.

No `cargo insta review` was run to reach this state; the snapshot was already accepted at some
point between this issue being filed and today, and the issue was simply never closed.

**The Hazard note above is retained deliberately.** Checking the snapshot size before measuring
the suite is cheap and still correct practice, even though the LFS cause is gone with `1383b84b`.
## Enum variant discriminants are not resolved per Rust's rules: explicit values don't advance the counter, and tuple variants fall back to the positional index

```issue
id: 09b82d44
kind: bug
severity: high
status: closed
```

### Summary

Three distinct defects in how enum variant discriminants are resolved. All are silent: a wrong
tag is written into the explicit-layout `Unmanaged` struct and read back by `ToManaged()`,
yielding the wrong variant rather than an error.

### Defect 1 — an explicit discriminant does not advance the counter

**Confirmed.** Reproduced 2026-08-25 in transaction `086e4102` by adding
`EnumExplicitThenImplicit { A = 5, B, C }` to the reference project. Generated C#:

```csharp
public static EnumExplicitThenImplicit A => new() { _variant = 5 };
public static EnumExplicitThenImplicit B => new() { _variant = 1 };
public static EnumExplicitThenImplicit C => new() { _variant = 2 };
```

`5, 1, 2`. Rust assigns 5, 6, 7. Marshalling `.B` sends 1, and Rust reads a variant that does
not exist at that tag.

`crates/proc_macros_impl/src/types/emit.rs`, lines 236-263:

```rust
let mut next_discriminant: isize = 0;
// VariantData::Unit
let disc = if let Some(expr) = &variant.discriminant { /* (#expr) as isize */ }
           else { next_discriminant };
next_discriminant += 1;   // never resumes from an explicit value
```

**This is the one that matters practically.** It needs only an explicit discriminant followed by
an implicit one on a plain unit-only enum — ordinary Rust. Defects 2 and 3 require the rarer
mix of explicit discriminants *and* payload variants.

**The wrong tag propagates to every consumer.** The same run shows `IsA`/`IsB`/`IsC`,
`AsA`/`AsB`/`AsC`, `ToString()` and `ExceptionForVariant()` all carrying `5, 1, 2`. They are
internally consistent, so no C#-side check can detect the fault; it surfaces only at the
boundary. That is the silent-corruption mode, now demonstrated rather than argued.

**Note on the fix.** `next_discriminant = disc + 1` is *not* implementable as written. The
explicit value is emitted as `(#expr) as isize` — a token stream evaluated at the call site,
not at macro-expansion time — so the macro never learns that `A = 5` is `5`. The fix is to
carry the previous discriminant as a `TokenStream` and emit implicit variants as `((#prev) + 1)`,
keeping evaluation at the call site where the expression is const-evaluable.

### Defect 2 — tuple variants discard their discriminant

Same site. The `VariantData::Tuple` arm increments the counter, then emits
`VariantKind::Tuple(<#ty as TypeInfo>::id())` with no tag. `variant.discriminant` is parsed and
available for tuple variants; the arm never reads it.

### Defect 3 — consumers substitute the positional index

Forced by defect 2: there is no tag to read. Each site does
`VariantKind::Tuple(t) => (index.cast_signed(), Some(*t))`.

| File | Line | Context |
|---|---|---|
| `backend_csharp/src/pass/model/common/types/kind/enum_variants.rs` | 54 | builds C# `Variant.tag` |
| `backend_csharp/src/pass/output/common/wire/mod.rs` | 318 | `emit_enum_serialize` |
| `backend_csharp/src/pass/output/common/wire/mod.rs` | 352 | `emit_enum_deserialize` |

Exhaustive within `backend_csharp`: `index.cast_signed()` occurs at exactly these three sites,
and `variants.iter().enumerate()` occurs at exactly the same three and nowhere else.

### Root cause

`crates/core/src/lang/types/enums.rs`:

```rust
pub enum VariantKind {
    Unit(isize),   // carries a discriminant
    Tuple(TypeId), // carries none
}
```

`Tuple` has nowhere to put a tag. The backends are doing the only thing available to them.

### Why `wire/mod.rs` must be fixed alongside the model pass

`wire/mod.rs` takes `e: &interoptopus::lang::types::Enum` — the **Rust inventory type**, not the
C# model's `DataEnum`. It bypasses `enum_variants.rs` and re-derives the tag independently.
Fixing the model pass alone leaves the wire path wrong.

The two currently agree, because both apply the same rule to the same variants in the same
order — so they are wrong together rather than inconsistent with each other. That is incidental,
not structural, and a shared unconditional tag is what would make it structural.

### Proposed fix

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

In the proc macro: resolve `variant.discriminant` in **both** arms, and set
`next_discriminant = resolved + 1`.

**Alternative considered and rejected:** `VariantKind::Tuple(TypeId, isize)`. Cheaper by four
one-line match arms, but keeps the discriminant conditional on payload shape — the exact shape
that produced three copies of defect 3 — and needs a third home when named and multi-field
variants land. Doing it that way first and moving to the field later costs strictly more, since
the `Unit(tag)` migration is still owed and `Tuple`'s arity churns twice.

### Blast radius (measured)

| Location | Change |
|---|---|
| `core/src/lang/types/enums.rs` | struct, `VariantKind`, `Variant::new` signature |
| `proc_macros_impl/src/types/emit.rs` | both arms + counter reset |
| `.../kind/enum_variants.rs` | 1 derivation, 1 mechanical arm |
| `.../output/common/wire/mod.rs` | 2 derivations, 2 mechanical arms |
| `.../output/common/wire/mod.rs:382` | payload-only match, mechanical only |
| serde inventory | shape changes under the `serde` feature |

The seven `.tag` reads under `output/common/types/enums/*` (`body`, `body_ctors`,
`body_tostring`, `body_unmanaged`, `body_to_unmanaged`, `body_as_unmanaged`,
`body_exception_for_variant`) consume the value `enum_variants.rs` produces and are
correct-by-construction once it is. No changes there.

`crates/_old/*` is **not** affected: it matches on `VariantKind::Typed`, which no longer exists,
so it is not compiled. An earlier revision of this issue wrongly listed `backend_c` and
`backend_cpython` as impacted.

### Verification

Reference-project additions, each with a round-trip assertion, and a wire round trip for the
enums that have one:

- `enum E { A = 5, B, C }` — unit-only, explicit then implicit → defect 1
- an enum mixing explicit discriminants with payload variants → defects 2 and 3

### Note

Blocks `docs/csharp-unions.md` item 4a, whose `ToManaged()` tag validation would otherwise
validate against the wrong tag set. Stands on its own as a correctness bug and should be
reviewed independently of that plan.
## MCP transaction commit cannot integrate any change: candidate tree bypasses the Git LFS clean filter

```issue
id: 1383b84b
kind: bug
severity: high
status: closed
```

### Symptom

`commit_transaction` fails after validation passes. The tool result carries only
`invalid_params/-32602: commit transaction failed`; the operation event log ends with a bare
`operation failed` and no error event. The real message is in the server log
(`Rust editor:get_server_log_tail`, filter `commit transaction failed`):

```
category: commit_pipeline_failed
detail: candidate commit would store content where a Git LFS pointer belongs:
        crates/backend_csharp/tests/backend_plugins/_plugins/exceptions.dll is
        24064 bytes in the candidate tree; committing it would silently un-LFS the path
```

Reproduced twice on transaction `086e4102381f1d60b16e9c35`, against target `2867011c` and
again after `pull_target` onto `4fac2166`. Identical detail both times. Re-measured later on a
fresh transaction carrying only its own ticket file, where it tripped on
`crates/proc_macros_impl/tests/snapshots/ty_enum__variants_negative.snap` instead — confirming
the guard reports whichever LFS path it reaches first, and that no change set is small enough
to avoid it.

### Diagnosis

The LFS **smudge** filter runs when the transaction worktree is created — content is
materialized correctly. `r#mod__reference_project__interop.snap` is 848,675 bytes in the
worktree, identical to the main checkout, and all 12 DLL-loading plugin tests pass there.

The LFS **clean** filter does not run when the candidate tree is built. Every LFS-tracked path
is therefore staged as raw bytes rather than as a pointer, and the commit guard correctly
refuses. This suggests the candidate tree is assembled by writing objects directly rather than
through `git add`, which would bypass `filter.lfs.clean`.

### Impact

**Blocked every transaction in this repository, regardless of what it changed.**

`.gitattributes` tracked nine extensions. The `deferred changes classified` event lists 68 such
paths; none belonged to the change under test. `exceptions.dll` was simply the first path the
guard tripped on. A transaction with an empty change set failed identically.

The guard was doing the right thing — silently un-LFS-ing 68 binary and snapshot paths would be
far worse than a failed commit. The bug is upstream of it.

### Resolution — closed by removing LFS, not by fixing the tooling

The underlying MCP defect is **not fixed**. It stopped mattering here because nothing in this
repository is LFS-tracked any more.

Untracking was worth doing on its own merits: of the nine tracked extensions, three were text
(`*.json`, `*.snap`, `*.svg`) and none were large — `global.json` is 63 bytes, the largest
committed DLL 30 KB. `.gitattributes` now classifies by text versus binary instead. The DLLs
left the repository entirely, since `define_plugin!` builds them during the test run.

Verified end to end: a transaction deleting the dead `build-dotnet-plugins` recipes built a real
candidate tree (`2b1825f7`) and integrated successfully. An earlier empty-change-set probe was
*not* sufficient evidence — a no-op never constructs a tree, which is exactly where this failed.

**Reopen this if LFS tracking is ever reintroduced.** The clean-filter bypass is still present
in the MCP commit pipeline and will resurface with the first LFS-tracked path.

Two smaller observations from the original investigation, still true:

- The failure detail is only reachable through the server log. `get_operation_log` omits
  `truncated_result`, so the operation log alone gives no actionable reason for the failure.
- `add_markdown_section` on the FileMcp server returns `No approval received` with no approval
  prompt shown to the user, while `replace_markdown_section` on the same file succeeds.

One further note, from closing this out: `commit_transaction` can return a transport-level error
to the client *after* the operation has already succeeded server-side. Check `get_commit_status`
before retrying a commit that appears to have failed.

## wire/mod.rs emits C# identifiers from the Rust inventory, bypassing union_names and names.rs

```issue
id: 4e9a17c3
kind: bug
severity: high
status: open
```

### Symptom

None yet. This is latent and will surface as a **compile error in generated `Interop.cs`**, in the
wire serializer, with no obvious connection to enum naming.

`crates/backend_csharp/src/pass/output/common/wire/mod.rs` emits C# identifiers derived from the
raw Rust inventory instead of the resolved C# model. Two families:

**Variant names — six sites**, all emitting members that `union_names` owns:

| Line | Emits | Should derive from |
|---|---|---|
| 320 | `{val}.Is{name}` | `stem` |
| 324 | `{val}.As{name}()` | `stem` |
| 361 | `{enum_name}.{name}({payload})` | `stem` |
| 363 | `{enum_name}.{name}` | `stem` |
| 385 | `{val}.Is{name}` | `stem` |
| 387 | `{val}.As{name}()` | `stem` |

`body.cs` and the other eight migrated consumers emit `Is{stem}`. When `stem != name` the two
disagree inside one generated file, and the wire call targets a member that does not exist.

**Type names.** `cs_type_name` returns `ty.name.clone()` for `Struct` and `Enum`. Every other
path through `names.rs` applies `sanitize_rust_name`, and most apply `rust_to_pascal` as well
(line 134 is the catch-all: `_ => sanitize_rust_name(&ty.name)`).

### Why it has not been caught

`stem == name` for every variant in the reference project, because nothing there collides —
exactly the blind spot `docs/csharp-unions-handoff.md` §3 records: *"no reference-project test
would catch it — nothing there collides."* Likewise `sanitize_rust_name` is identity for ordinary
Rust identifiers, so the type-name divergence is invisible on the current corpus.

The plan's Step 1 claims **all nine** name-deriving sites emit from `v.stem`. This is a tenth.
Item 1c is therefore incomplete.

### Not licensed by the Wire comment

`names.rs:113` says *"The inner type of Wire may not have a C# TypeKind (its fields use WireOnly
types), so resolve the name from the Rust inventory directly."* That licenses reading the source
string from `rs_types` — and the very next lines still apply `sanitize_rust_name` and
`rust_to_pascal` to it. Resolving *from* the Rust inventory is not the same as using the Rust name
*verbatim*. Wire does the latter.

### Root cause

`WireCodeGen` holds only the Rust inventory:

```rust
pub struct WireCodeGen<'a> {
    pub rs_types: &'a RsTypes,
}
```

There is no path from `self` to the C# model, so no emitter in this file can reach a resolved
name. `emit_enum_serialize` already receives a `TypeId` it ignores (`_ty_id`), which is the
natural hook once a model reference exists.

This is the same disconnection that produced the tag defect in `09b82d44`, whose blast radius
described these sites as re-deriving "independently of the model pass". That fix corrected the
tags at `wire:318` and `wire:352` without addressing why wire was re-deriving at all.

### Trigger date — this fires as Step 3 lands

Divergence is rare today and becomes common:

- **3b** adds nested `{stem}Case` types, a new collision class against existing members.
- **3c** reserves `Value`, `HasValue`, `TryGetValue`; variants named those get moved to fallback
  stems.
- The allocator is preservation-biased, so a moved variant is precisely the `stem != name` case.

**Item 5c is the test that catches it.** A collision case whose stem differs from its name will
produce inconsistent output across `body.cs` and wire. Worth writing 5c early for that reason.

### Third, separate alignment gap

`emit_enum_serialize` branches on `IsX` and falls through to
`throw new InvalidOperationException("Unknown variant")`. Once item 3c consumes `_hasValue`,
every `IsX` returns false for a default struct union, so wire reaches that fallback for an empty
enum. Step 4 specifies `InvalidOperationException` for empty-state marshal-out and
`InteropException` for a corrupt tag, both routed through `ExceptionForVariant()`. Wire honours
neither and hand-rolls its own message.

### Proposed fix

Give `WireCodeGen` a reference to the resolved C# names and use it for both families, then align
the exception path with Step 4. That means a new field and touching every construction site;
scope it before starting.

Rejected: putting `stem` on the core `interoptopus::lang::types::Variant`. A stem is the output
of C#-specific collision resolution (CS0102, CS0542, C# reserved words) and does not belong in
the language-neutral core, which `backend_c` and `backend_cpython` also compile against. It is
also unknown at proc-macro time — unlike `tag`, which the macro emits — so a core field would be
constructed empty and filled by one backend, with nothing preventing a second from filling it
differently.

### Unmeasured

Whether `cs_type_name`'s `_ => "object"` fallback ever fires for a legitimate `Wire<T>` payload.
Wire fields are `WireOnly` by construction, which may be exactly the subset the function handles.
A read, not a change.
### Design — agreed

**Implemented, and partly superseded by what was built.** Two things below no longer describe the code. (1) The prebuilt `WireNames` map, its builder and its `&str` strictness: replaced by a two-method resolver with lazy `Option` lookups and two panicking accessors at the call site — there is no map, so there is no domain to define, which was the only real objection to the original design. (2) The `rust_to_pascal(sanitize_rust_name(&ty.name))` fallback for a type with no C# mapping: dropped entirely. A locally derived name is a second naming authority, which is the defect family this issue is about; a missing model entry is now a panic. The rest of this section stands as written, including the two rejections and the `(TypeId, tag)` keying.

#### Resolve at the pass boundary, not inside the emitter

`WireCodeGen`'s methods return `String` and `()`, not `Result`. Making resolution strict *inside*
them would mean threading `Result` through ten mutually recursive methods — `cs_type_name`, the
three `emit_*` walkers, the three enum emitters and the three struct-body helpers — which is a
large diff for a naming fix.

Both construction sites are already inside `process()` functions returning `OutputResult`, with
`id_map` in scope. Build and validate the lookup there; hand the emitter a prepared map.

#### New type: `WireNames`

Lives in `pass/output/common/wire/` — it is wire's view of the model, not a model concern.

```rust
pub struct WireNames {
    types: HashMap<TypeId, String>,             // Rust TypeId -> canonical C# name
    variants: HashMap<(TypeId, isize), String>, // (Rust enum TypeId, tag) -> stem
}
```

Keyed by **Rust** `TypeId`, because that is what wire holds while walking the Rust graph.

Variants key on **`(TypeId, tag)`**. Not positional index — that breaks under any future variant
filtering, which several output passes already do. Not name — that is the thing being corrected.
`tag` has been the shared authoritative discriminant on both sides since Step 0 (`c928d53e`), so
it is the one identity both models agree on.

#### Builder

`WireNames::build(id_map, names, kinds, rs_types, enums) -> Result<Self, Error>`

For each enum the pass is about to emit:

- `id_map.ty(rust_id)` → cs id, else `Err` naming the Rust type
- `names.get(cs_id)` → canonical type name, else `Err`
- `kinds.get(cs_id)` → the C# `DataEnum`, else `Err`
- each C# variant: insert `(rust_id, variant.tag) -> variant.stem`; `Err` if `stem` is empty

An empty stem is `union_names`' own "unresolved" sentinel — the same convention it uses for
convergence — so this reuses an existing invariant rather than inventing one.

#### Strictness

Lookups on the built map return `&str`, **not** `Option<&str>`. This is deliberate: an `Option`
here invites `unwrap_or(&variant.name)`, which would silently reinstate exactly this bug while
looking like defensive coding. Every failure mode is caught in the builder, where `Err` is
returnable and the message can name the enum and tag.

#### `WireCodeGen`

```rust
pub struct WireCodeGen<'a> {
    pub rs_types: &'a RsTypes,
    pub cs: &'a WireNames,
}
```

Method signatures unchanged. The diff is the six variant sites plus two `cs_type_name` arms.

#### The six variant sites

All in `wire/mod.rs`: `emit_enum_serialize` (320, 324), `emit_enum_deserialize` (361, 363),
`emit_enum_size` (385, 387). Each `variant.name` becomes `self.cs.variant_stem(ty_id, variant.tag)`.

`emit_enum_serialize` already receives the enum's `TypeId` as `_ty_id` and ignores it — drop the
underscore. Confirm the other two receive it; thread it if not.

#### `cs_type_name`

```rust
RsTypeKind::Struct(_) => self.cs.type_name(ty_id),   // was ty.name.clone()
RsTypeKind::Enum(_)   => self.cs.type_name(ty_id),   // was ty.name.clone()
```

Everything else unchanged. `WireOnly` composition — `List<T>`, `Dictionary<K,V>`, `T?`, `T[]` —
stays recursive here: those are *shapes* derived from the Rust graph, not nominal identifiers, and
walking the Rust graph is correct for them because `WireOnly` fields have no C# `TypeKind`.

This preserves the split worth keeping: **the Rust graph decides traversal and composition; the C#
model decides canonical identifiers.**

For a type with genuinely no C# mapping, the builder falls back to
`rust_to_pascal(sanitize_rust_name(&ty.name))` — the same helpers `names.rs:118` uses, called
rather than copied.

#### Threading

Two construction sites, both already receiving `id_map`:

- `wire/helper_classes.rs:40` — `let codegen = WireCodeGen { rs_types };`
- `wire/wire_type.rs`

Their `process()` signatures gain the names and kinds passes. Four pipeline call sites (Rust and
dotnet), which already have `m.type_names` and `m.type_kinds` in scope. Two signatures and four
calls — not architecture.

#### Tests — item 5c, extended

1. **Colliding variant** (`Foo` / `IsFoo`) forcing `stem != name`. Assert all six wire emissions
   use the stem. Without this the defect reopens silently.
2. **A type whose `sanitize_rust_name` is not identity, reachable inside a `Wire<T>`.** Assert wire
   emits the sanitized C# name.

Together these prove both families, not just the variant half. Both are acceptance gates for 1d.

#### Explicitly out of scope

- Replacing `_ => "object"` with a generation error. Separate change; a snapshot search for
  `object result`, `List<object>`, `WireOfObject` and `public required object` found nothing, so
  there is no evidence the path fires today. Worth doing eventually — a generator emitting `object`
  because it does not understand a type is masking a model bug — but it does not gate 1d.
- Deduplicating the `WireOnly` composition that `names.rs` (87-97) and `cs_type_name` (34-43) both
  implement. Real duplication and already present; the two compose from different resolvers, so a
  shared helper takes a resolver parameter. Factor when a third consumer appears or when the two
  drift, not now — but note it here so the drift is not rediscovered.
- Item 4d, wire's exception contract. Same file, different concern, gated on 4b.

#### Order

`WireNames` + builder → threading → six variant sites → `cs_type_name` → 5c → then 3b.
## Enum variant names are never sanitized, so a C# keyword variant emits uncompilable bindings

```issue
id: 7c8cb22e
kind: bug
severity: low
status: closed
```

### Symptom

A Rust enum variant whose name is a C# reserved keyword emits invalid C#:

```rust
#[ffi]
pub enum E {
    class,
    event,
}
```

produces

```csharp
public static E class => new() { _variant = 0 };
public bool Isclass => _variant == 0;
```

`class` is a C# reserved keyword and cannot be an identifier, so the generated bindings do not
compile. The generator reports no error.

### Why variants but not type names

Type names go through `sanitize_rust_name` (`crates/backend_utils/src/casing.rs`), which
PascalCases the result. Since every C# reserved keyword is lowercase, a Rust type named `class`
emits `Class` and is incidentally safe.

Variant names take a different path and are never sanitized:

- `crates/backend_csharp/src/pass/model/common/types/kind/enum_variants.rs` does
  `name: rust_variant.name.clone()`
- the enum templates emit `{{ v.name }}` verbatim

So variants are the only identifier class in the C# backend with no keyword protection.

### Scope

Requires an unconventional Rust variant name: Rust style is PascalCase, and `Class` is not a C#
keyword. Legal Rust, and it silently produces broken output, but unlikely to be hit by accident.
Affects reserved keywords only — contextual keywords such as `type`, `value` and `record` are
valid C# identifiers in these positions.

Escaping with `@class`, or PascalCasing variant stems, would both fix it.

### Explicitly not fixed by `union_names`

`pass/model/common/types/union_names.rs` allocates collision-free variant names, but only against
*other generated members* and the fixed union contract. It deliberately keeps stems **verbatim**,
because the templates emit them verbatim today and re-casing would rename members on enums that
have no collision at all. Keyword escaping is a separate concern and was left out of that pass on
purpose; see `docs/csharp-unions.md` item 1b.

For contrast, three collision classes that *are* fixed by `union_names` once the templates are
migrated (item 1c) — `Foo`/`IsFoo`, `Foo`/`AsFoo`, `Foo`/`UnmanagedFoo`. Those are also
pre-existing breakage in the current generator, since `IsX`, `AsX` and `Unmanaged{X}` are already
emitted; they are noted here only so the distinction is on record.

### Verification

A reference-project enum with a keyword variant would need the fix in place first, since adding
one now would break the build rather than a test. A `union_names`-level unit test asserting the
escaped or cased stem is the cheaper gate.

### Resolution

Closed as non-reproducible on the supported `#[ffi]` path. The attempted reference-project
fixture failed during proc-macro expansion with `Using the name 'class' can cause conflicts in
generated code.` `TypeModel::validate_forbidden_names` checks every enum variant against
`FORBIDDEN_NAMES`; a programmatic comparison confirms all 77 reserved C# keywords are already
present in that 136-name cross-backend list. The variant therefore never reaches `TypeInfo`, the
C# model, or an emitter.

No C# generator change was retained. A manually implemented unsafe `TypeInfo` could fabricate an
inventory that bypasses the proc macro, but supporting such manually constructed keyword names is
a separate contract and was not the reported defect.

## wire's is_cs_value_type re-derives struct-vs-class that struct_class::Pass owns

```issue
id: 31248473
kind: bug
severity: medium
status: closed
```

### Symptom

None yet. Latent, and a **compile error in generated `Interop.cs`** when it fires — `'X' does not contain a definition for 'HasValue'`.

### What it does

`crates/backend_csharp/src/pass/output/common/wire/mod.rs::is_cs_value_type` decides whether a Rust type maps to a C# value type, which drives the `Option` branching in `emit_option_serialize`, `emit_option_deserialize` and `emit_option_size` — `.HasValue`/`.Value` for a value type, a null check for a reference type.

It decides this by walking the Rust graph:

```rust
RsTypeKind::Primitive(_) | RsTypeKind::Enum(_) | RsTypeKind::Array(_) => true,
RsTypeKind::Struct(s) => !s.fields.iter().any(|f| contains_wireonly(f.ty, rs_types, ...)),
```

But `pass::model::common::types::info::struct_class::Pass::is_struct` already owns that decision, and six output passes consult it — `composites::{body, body_unmanaged, definition}` and `enums::{body, definition}` among them. Wire is the one that re-derives it.

### Two concrete divergences

**`Enum(_) => true` unconditionally.** `enums/body.rs:79` picks `struct` or `class` via `struct_class.is_struct(ty)`. A `DataEnum` emitted as a class then gets `.HasValue`/`.Value` from wire's `Option` path — a reference type has neither.

**`Array(_) => true`.** C# arrays are reference types. `Option<[T; N]>` inside a wire payload emits `.HasValue` on a `T[]`.

Whether either shape occurs in a real inventory today is **unmeasured**. `Option<DataEnum>` inside a `Wire<T>` is the likely first one.

### Same family as `4e9a17c3`

An emitter re-deriving something the model already owns. That issue fixed the two naming families in this file; this is the third thing the same file re-derives, and unlike them it is not about identifiers, so it survived that fix untouched.

Counting the whole family: `tag` re-derived three times (`09b82d44`, `c928d53e`), names twice in wire (`4e9a17c3`), names once inside the model (`wire::nested`, fixed alongside 1d). This would be the seventh. The recurrence is the finding — it argues for handing passes capability-narrow accessors rather than whole model handles.

### Proposed fix

Thread `struct_class::Pass` to `WireCodeGen` the way `4e9a17c3` threaded the name resolver — as a narrow accessor, not the whole pass — and delete `is_cs_value_type`'s own derivation. Note that `contains_wireonly` is currently duplicated between this file and `types/kind/struct_fields.rs`; the model-side copy is the authority.

### Also here, unrelated to the above — RETRACTED

This issue claimed `wire/helper_classes.rs::resolve_field_type_name` held "two policies for one failure". **That was wrong, and it was wrong when filed.** The two returns answer different questions. The loop asks which Rust type maps to `cs_ty` and hands the answer to wire, where a missing model entry is an invariant violation and `cs_type_name` panics. The fallback fires when *no* Rust type maps to `cs_ty` at all — which `types::all` documents as normal for synthesized types such as overload siblings. Different conditions, both behaviours correct.

No code change. The fallback's condition was simply never stated, which is what made it read as duplicated error handling; it is now documented in place.

### Resolution

Reproduced before fixing, which is what the ticket demanded. An enum with a `String` payload is `Into`, so `struct_class` emits it as a class; `Option<Choice>` inside a `Wire<T>` produced eight uncompilable lines:

```csharp
writer.Write((byte)(value.choice.HasValue ? 1 : 0));
if (value.choice.Value.IsText)
_size += 4 + ...GetByteCount(value.choice.Value.AsText() ?? "");
```

Not latent, and worse than predicted — the prediction was the `HasValue` check, not `.Value` threaded through every payload access.

The fix is a hybrid, forced by measurement rather than chosen: `struct_class.is_struct` answers `false` for anything unregistered, so delegating a primitive would report "reference type" and turn `Option<u32>` into a null check on a `uint?`. Primitives and `WireOnly` are still answered locally; only `Struct` and `Enum` go to the model, through a new `CsLayout` — a second narrow view rather than a third method on `CsNames`, since struct-vs-class is not an identifier concern.

`Array(_) => true` was corrected to `false` in passing. C# arrays are reference types. No snapshot moved, so nothing in the corpus exercises it: a latent fix, unverified by test.

Mutation-proven: reverting the delegation reproduces the same eight lines and fails only its own test. Regression test `tests/output/wire/option_value_type.rs`, with a guard asserting the fixture is still class-backed.

**Timing mattered.** After Step 3c a class-backed union declares its own `HasValue`, so `.HasValue` would bind to the union contract's constant-`true` member rather than failing to compile — turning a loud error into a silent one. Fixed before 3c for that reason.

## wire::nested writes type names, making it a second naming authority alongside names.rs

```issue
id: c33b9cf5
kind: issue
severity: low
status: closed
```

### Symptom

None yet. `wire::nested` no longer produces an invalid identifier — that was fixed alongside item 1d — but it is still a second writer to `type_names`, which is the shape of the defect rather than its symptom.

### What happened

`pass/model/common/wire/nested.rs` registers structs that transitively contain `WireOnly` fields, because `struct_fields.rs` deliberately skips them. It registered both the kind **and** the name:

```rust
type_kinds.set(cs_id, TypeKind::WireOnly(CsWireOnly::Composite(composite)));
type_names.set(cs_id, rust_ty.name.clone());   // raw inventory string
```

`names.rs:72` is first-write-wins (*"Skip if we've already mapped this name"*) and `nested` won, so the raw string became the model's answer — even though `names.rs:98` has an arm for exactly this kind that applies `sanitize_rust_name`.

For an instantiated generic the raw string is not a legal C# identifier. Measured, before the fix:

```csharp
    public required Boxed<u32> inner;
        result.inner = (Boxed<u32>)...GetUninitializedObject(typeof(Boxed<u32>));
public partial class Boxed<u32>
```

A live compile error in generated code, one of them a class declaration emitted by `helper_classes` from the model name — so not a wire defect at all. It surfaced only because item 1d routed wire through the model, making the model's own answer observable.

### What was fixed, and what was not

Fixed: `sanitize_rust_name` applied at both write sites, matching `names.rs:98`. Called, not copied. Regression test: `tests/output/wire/nested_composite_names.rs`.

Not fixed: `nested` still writes names. That is the actual defect. `names.rs` is the naming pass; a kind-registration pass deciding names is how two authorities appear in the first place, and the next name transform added to `names.rs` will silently not apply to this family.

### Proposed fix

Drop the `type_names.set` calls and the `type_names` parameter from `nested::process`, letting the convergence loop carry the name from `names.rs` on the next iteration.

Two things to check first, because they are why this was not done inline:

- **Ordering.** With first-write-wins, confirm `names.rs` does not set an early name from the *Rust* kind before the composite kind exists. If it does, that early name is `sanitize_rust_name(&ty.name)` — the same answer — so it is likely harmless, but it should be established rather than assumed.
- **Convergence.** `nested` is a run-once pass (`self.done`). Removing a write changes what it reports as `Changed`; make sure the loop still terminates.

Both pipeline call sites change, which is why it was out of scope for a naming fix.

### Resolution

Both preconditions checked by reading before any edit, as this issue demanded.

**Ordering — clear, and for a better reason than expected.** `names.rs` keys off the *C# kind* (`kinds.get(cs_id)`), never the Rust kind, so it cannot name a composite before this pass registers one. There is no early write to race against, so first-write-wins never applies here.

**Convergence — clear, and the concern was overcautious.** This issue worried that dropping a write would change what the pass reports as `Changed`. It does not: `outcome.changed()` follows the `type_kinds.set`, not the name write. Removing the latter leaves the signal, and therefore the loop, untouched.

Dropped both `type_names.set` calls and the parameter; both pipeline call sites updated. `names.rs` line 98 is now the sole authority for these names.

**The coverage moved rather than evaporated**, which is the part worth proving. Breaking `names.rs`'s `WireOnly::Composite` arm now fails `nested_wire_composites_get_a_valid_csharp_identifier` with *"the raw inventory name reached the generated C#, which does not compile"*. Before this change that same mutation passed, because `nested` supplied the name and `names.rs` never ran for these types. Restored byte-identical afterwards.

Snapshots unmoved.

### Related

`4e9a17c3` — the same family, one layer up. `31248473` — the same file, a third thing re-derived. With this closed, every known instance of the re-derivation family is fixed.
## _plugins staging omits NuGet dependencies, so wire::load_plugin is a false green on any fresh checkout

```issue
id: e235bc7d
kind: bug
severity: medium
status: closed
```

### Symptom

`reference_plugins::wire::load_plugin` fails on a fresh checkout and passes on one that has run it before, with no source difference between them:

```
FileNotFoundException: Could not load file or assembly
  'Newtonsoft.Json, Version=13.0.0.0, Culture=neutral, PublicKeyToken=30ad4fe6b2a6aeed'
  Requested by: wire, Version=1.0.0.0
   at My.Company.Plugin.WireString(WireOfString nested)
```

The project builds cleanly — `Build succeeded, 0 Errors`. It fails at assembly load.

### Cause

`tests/reference_plugins/wire.dll/wire.csproj` has a real dependency:

```xml
<PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
```

and `Plugin.cs` calls `JsonConvert` in `WireString`, which is the failing frame.

`define_plugin!` builds the `.csproj` and stages the plugin's own DLL into `_plugins/`, but nothing stages the DLL's *dependencies*. On the main checkout `tests/reference_plugins/_plugins/Newtonsoft.Json.dll` exists — 712 KB, dated **2026-08-14** — left behind by an older run. On a fresh worktree there is no such file anywhere under `tests/`, and the test fails.

Measured both ways: fails twice in a fresh worktree, passes in the main checkout, and the only difference is that stale artifact.

### Why it matters

The test currently reports green because of a file no build step produces. Any clean clone, any CI runner without a warm working tree, and a worktree-based workflow all hit it. It is a false green, and it is the only plugin test with a third-party `PackageReference`, so nothing else in the suite covers the gap.

This is adjacent to what `9d664613` removed: plugin DLLs used to be committed, and `define_plugin!` building them at test time is what replaced that. Dependency staging did not come along.

### Proposed fix

Stage the build output directory rather than the single DLL — copy the `.csproj`'s resolved runtime assets into `_plugins/` after build, not just `$name.dll`. `ensure_plugin_built` is the natural place, since `define_plugin!`, `load_plugin!` and `dll_path_for` all route through it (`docs/csharp-unions-handoff.md` §7).

Verification is cheap and specific: delete `tests/reference_plugins/_plugins/Newtonsoft.Json.dll` in the main checkout and confirm the test fails before the fix and passes after. Do not verify on a tree that has run the suite before.

### Resolution — closed by removing the need, not by implementing the fix

The dependency itself was unnecessary. `Plugin.cs` used `JsonConvert` only to round-trip a
`Dictionary<string, string>` through a string; `System.Text.Json` does that identically and is
in the BCL, so nothing needs staging. The fixture was ported, both `PackageReference`s were
removed — the second, in `Tests.csproj` at a different version, had zero usages in any `.cs`
file — and `Newtonsoft.Json` is now absent from the repository.

Building a dependency-staging mechanism to support one fixture's convenience import would have
been engineering around a problem instead of deleting it. The staging gap is real and would
return the moment a plugin fixture takes a third-party `PackageReference` again; that
constraint is now recorded in `docs/csharp-unions-handoff.md` §7 rather than defended by code.

### Note

Found while measuring 1d's test failures (`4e9a17c3`). It was the one failure that did not clear
on a re-run, which is what separated it from the six first-run plugin-build ordering failures
around it.

Those six, and a later run of nineteen, turned out to be two further and entirely separate
defects — neither in interoptopus. See the transaction that closed this issue.

## Unit-only enums carried full union machinery; eligible cases now emit plain enums

```issue
id: 79be256e
kind: issue
severity: medium
status: fixed
```

### The shape

**Before option C landed**, a Rust enum with no payload-carrying variant was projected as a C# **struct** with a discriminant field, a custom marshaller and an `Unmanaged` mirror. The following snapshot from `EnumDocumented { A, B, C }` records the old shape — three symbols, no data:

```csharp
public partial struct EnumDocumented { byte _variant; bool _hasValue; }

[NativeMarshalling(typeof(MarshallerMeta))]
public partial struct EnumDocumented
{
    [StructLayout(LayoutKind.Explicit)]
    internal unsafe struct Unmanaged { [FieldOffset(0)] internal byte _variant; internal EnumDocumented ToManaged() {...} }
    internal Unmanaged ToUnmanaged() {...}
    internal Unmanaged AsUnmanaged() {...}
    public Exception ExceptionForVariant() {...}
    public static EnumDocumented A => new() { _variant = 0 };   // x3
    public bool IsA => _variant == 0;                            // x3
    public void AsA() { if (_variant != 0) throw ExceptionForVariant(); }  // x3
    public override string ToString() {...}
    private struct MarshallerMeta { }
    internal ref struct Marshaller {...}
}
```

About 120 lines. Union projection (items 3b, 3c) then adds a nested case type per variant, `Value`, `HasValue` and `TryGetValue` on top.

### The governing rule is anti-bloat, not the count

A unit-only enum has no payload. Its case types are empty, its `Value` is nothing, and
`TryGetValue` has nothing to get. Emitting that machinery is generating structure for a type
that has no use for it, and that holds for one such enum as much as for a hundred.

**The population count is evidence of payoff, not justification.** An earlier version of this
issue reasoned the other way and reached the wrong conclusion by doing so; see below.

### How much of the population this is

**Reference project: 4 of 6.** `EnumDocumented`, `EnumRenamedXYZ`, `EnumNegative`,
`EnumExplicitThenImplicit` are unit-only; `EnumPayload` and `EnumExplicitPayload` carry payloads.
`docs/csharp-unions.md` notes `_hasValue` currently trips CS0169 on **eight** generated types.

**gitoxide `gix` crate — measured, and it corrects the earlier claim.** A syn visitor over
`gix/src` (251 files, `pub` enums, excluding those named `Error`) classified 36 enums:

| Class | Count | Representable |
|---|---|---|
| Pure unit | 15 | 15 |
| Mixed | 4 | 2 |
| Pure payload | 17 | 6 |
| **Total** | **36** | **23** |

"Representable" excludes what `VariantKind::Tuple(TypeId)` cannot carry — named-field variants,
multi-field tuples, generics.

**Unit-only is therefore 15 of 23 (65%), or 15 of 21 (71%) once `AsError` and `CleanupError` are
dropped as error enums the name filter missed.** A previous version of this section reported
"8 unit-only, 12 with payload — so *not* 'mostly unit-only' as first assumed" from a 20-enum
sample filtered to files named `types.rs`. That sample counted the unrepresentable population
alongside the representable one. **The first assumption was right; the sample misled it.**

Caveat on scope: this is the `gix` porcelain crate only, not the `gix-*` plumbing crates or
`gitoxide-core`, and **no `#[ffi]` annotation exists anywhere in gitoxide** — so it is a
projection of what binding it would produce, not a count of what is bound. A count over another
wrapped crate would change the payoff. It would not change the rule.

### The public surface changed deliberately

The landed design is a plain C# enum, **without** an extension block that recreates the old
struct API. That is the breaking surface recorded in the backend changelog:

| old struct API | plain-enum API |
|---|---|
| `x.IsA` | `x == E.A` |
| `x.AsA()` | compare or switch on the enum member |
| static factory property | enum member |
| custom `ToString()` | the enum's built-in member name |

The removed mirror, marshaller and conversion helpers were private, internal or hidden from
IntelliSense. The public `IsX` / `AsX` surface was not preserved; consumers migrate to ordinary
enum operations. Out-of-range values now stringify numerically instead of throwing from the old
custom `ToString()`, consistent with ordinary C# enum behavior.

### THE VALIDATION ARGUMENT IS FALSE — correcting the record

An earlier draft of this reasoning kept the struct on the grounds that it validates unknown native discriminants. It does not. `Unmanaged::ToManaged` is:

```csharp
var _managed = new EnumDocumented();
_managed._variant = _variant;
return _managed;
```

The tag was copied blind. The only `InteropException("Illegal enum state detected")` was in `ExceptionForVariant()` and `ToString()` — the error-mapping and display paths, not marshalling. Item 4a has since landed for union-projected structs. Plain enums still perform no tag validation, which is the documented exhaustiveness trade-off accepted by option C.

### The lever — DISPROVED, see `5d1ae4c7`

This section originally reasoned that `managed_conversion` is the single lever: that
`composites/body_unmanaged.rs` does not hardcode the mirror, that field types and conversions come
from passes rather than string-appending `.Unmanaged`, and therefore that classifying a unit-only
`DataEnum` as `AsIs` would make the downstream fall out.

The first half is true and still useful — a containing struct does ask the model. **The conclusion
is false.** Traced in `5d1ae4c7`: every enum output pass gates on `TypeKind::DataEnum` alone, in
ten inline copies of the same three-arm match, none of which consults `managed_conversion`,
`struct_class`, or variant shape. Reclassifying moves struct-vs-class and containing-type
conversion; `body_unmanaged.rs`, `body_unmanaged_variant.rs`, `body_to_unmanaged.rs` and
`body_as_unmanaged.rs` would each still fire.

The real emission-suppression seam is `is_managed_only` in `enums/body.rs`, a single site — but it
is the wrong predicate to extend, since it means *never crosses FFI* and a plain-enum projection
crosses FFI precisely because it is blittable. See `5d1ae4c7` for the third-category question that
follows.

**Historical note:** those inline site counts were snapshots taken before the projection pass.
The shared projection model and the later plain-enum implementation replaced the need to infer
representation by counting output-pass matches; the old ten/eleven/twelve figures are not current
architecture.

### Historical open questions — resolved or separated

The questions below were useful before the implementation, but they are no longer blockers:

- The representation seam is the shared projection model, not a local `managed_conversion` guess
  or a count of enum emitters.
- The full generated C# consumer gate proves the landed plain-enum route compiles through Wire,
  composites and slices; the abandoned extension-block design is no longer relevant.
- Reducing `union_names`' reserved family for unit variants remains a separate naming/API-stability
  trade-off. It was not required for option B or C and is not part of this closed issue.

### Three options — final status

**A. Status quo — superseded.** It kept the full struct and union machinery for every
`DataEnum`.

**B. Skip union machinery for unit-only enums, keep the struct — landed.** The payload-capability
gate removed case types, `Value`, `HasValue` and `TryGetValue` before the representation
change.

**C. Emit a plain C# enum when its discriminant is a legal enum base — landed in `ca6aafa`.**
This removed the old struct surface, mirror and marshaller for the eligible population. Commit
`d29fa975` pins the exception: manual inventory metadata with an `isize` / `usize`
discriminant maps to `nint` / `nuint` and remains a discriminant struct because C# forbids
native integers as enum bases.

### Closed enums were not a prerequisite

Closed enums would restore compile-time exhaustiveness and reject arbitrary integral casts, but
they did not ship in C# 15. The implementation did not wait for them: it accepted the ordinary C#
enum trade-off, documented the breaking API migration, and kept runtime/native layout exact.
A future closed-enum feature may improve the managed surface; it is not a gate for the landed
projection.

### Two rules, now measured

Option B follows the anti-bloat rule: a type with no payload-capable variant receives no union
machinery.

Option C rests on a separate ABI rule: a unit-only enum may become a plain C# enum only when its
inventory discriminant maps to a legal C# enum base. The supporting claims are now measured:

- the old unit-only `Unmanaged` mirror contained only the discriminant at offset zero;
- `#[ffi]` derives Rust repr and inventory layout from the same fixed-width choice;
- pinned arrays of the generated plain enum survive the boundary;
- manual `Isize` / `Usize` inventory metadata maps to `nint` / `nuint` and therefore keeps
  `Projection::Discriminant`.

The last point is generator-level coverage rather than a reference-project runtime fixture:
`#[ffi]` deliberately normalises enum reprs to fixed widths.

### B and C landed — issue closed

The payload-capability gate for B landed in `4a19b0e3` and `bdd13b53`, consolidated through
`DataEnum::is_union_projected()` in `98f7ffd7`.

Option C landed in `ca6aafa`: eligible unit-only enums are ordinary C# enums with the matching
underlying type, and no longer expose the prior struct accessors or custom marshaller.

Commit `d29fa975` closed the remaining generator-coverage gap. Its test-only
`PointerSizedFlag` is registered directly as manual inventory metadata with
`Layout::Primitive(Isize)`; the focused test asserts the managed struct, `nint` unmanaged
discriminant and marshaller remain, while plain-enum and union machinery do not appear. Direct
registration is intentional because ordinary `#[ffi]` enums cannot produce pointer-sized reprs.

Validation at `d29fa975`: the focused fixture passed; the complete
`interoptopus_csharp` Cargo test package passed **63 tests** with **2 intentional ignores**; the
embedded .NET suite passed **223/223**.

`docs/csharp-unions.md` § Todo/Remaining is the status of record and carries the same two-way
projection rule.

### Related — folding mixed enums, DROPPED

The idea: for a *mixed* enum, fold its unit variants into one nested C# enum used as a single
union case, leaving case types only for payload variants. Dropped on three grounds, one of them
measured.

**Measured: it has almost no population.** Of the four mixed enums in `gix`, only two are
representable — `TrackRenames` (2 unit of 3) and `SubSectionRequirement` (1 unit of 2). Folding
would apply to two enums and save three case types. The best candidate by far, `Mode` in
`remote/connection/fetch/update_refs/update.rs` at 8 unit variants of 10, is **not**
representable: its two named-field variants cannot reach the backend.

**It weakens exhaustiveness rather than strengthening it.** A switch over the folded union sees
two cases — the nested enum and the payload case — and is exhaustive once both are covered, even
though the folded variants have not been distinguished. Telling them apart requires an inner
switch over a plain enum, which is exactly the case C# does *not* exhaustiveness-check. Folding
moves variants out of the checked layer into the unchecked one.

**Tag provenance.** The union's case identity would no longer determine `_variant` for the folded
group, so the tag is recovered from two different places depending on the case — the defect family
behind `09b82d44` and `c928d53e`.

The `IsX`/`AsX` objection recorded earlier is *not* among the reasons; it rested on preserving
public API, which is a weaker constraint where the consumers are owned.

~~nullable.rs classifies only class delegates as nullable, so every other reference type gets an unguarded conversion~~ — fixed in `c56f941`

```issue
id: b4e07f12
kind: bug
severity: medium
status: fixed
```

### Symptom

`pass/model/common/types/info/nullable.rs` documents itself as deciding "whether a type is
nullable in C# (i.e., a reference type / class)". Line 35 implements something much narrower:

```rust
let is_nullable = matches!(&ty.kind, TypeKind::Delegate(d) if d.kind == DelegateKind::Class);
```

Class **delegates** only. Every other C# reference type is reported non-nullable.

The two consumers, `body_as_unmanaged.rs:47` and `body_to_unmanaged.rs:48`, use that answer to
choose between a guarded and an unguarded field conversion, emitting `?{suffix} ?? default` when
nullable and a bare suffix otherwise. So a composite field whose type is a reference type other
than a class delegate gets an unguarded `.AsUnmanaged()` / `.ToUnmanaged()`.

### This is not conditional on 3a

Class-backed enums exist today, before any union work. `struct_class` emits an enum as a class
whenever its managed conversion is `Into`; `tests/output/wire/option_value_type.rs` records
exactly that for `Choice`, which carries a `String`, and names `OptionUtf8String` and the owned
`Result` types as the same category. `is_cs_value_type`'s doc-comment adds that structs with
`WireOnly` fields are emitted as classes.

The mis-classified set today therefore already includes class-backed enums and class-backed
composites. 3a widens it; it does not create it.

### Measured, and not measured

Measured statically: the classification at `nullable.rs:35`, both emission sites, and the
class-backed-enum category via `struct_class` and `option_value_type.rs`. **Not executed** — no
test yet reproduces a runtime `NullReferenceException` on a null class-backed field.

### Same family as 31248473

`31248473` closed wire's `is_cs_value_type` re-deriving struct-vs-class, a question
`struct_class::Pass` owns. `nullable.rs` is another re-derivation of that same question in a
different vocabulary — "is this a reference type" — and it is stale in the same way.
`struct_class::is_class` already exists at `struct_class.rs:55` and is the authoritative answer.

~~Do not close this before `docs/csharp-unions.md` open item 1~~ — satisfied, fixed in `c56f941`

The warning existed because the obvious fix — delegate to `struct_class::is_class` — would have
put class-backed unions on the `?? default` branch, yielding a zeroed `Unmanaged`: discriminant 0,
a fabricated variant crossing FFI, silently. Deciding that by accident is what it guarded against.

**It was satisfied rather than waived.** Open item 1 closed with `InvalidOperationException`, and
class-backed unions are now excluded from `?? default` **explicitly**, via `NullPolicy::Throw`.
`nullable.rs` therefore asks `struct_class` instead of re-deriving reference-ness from `TypeKind`,
ending the duplication `31248473` closed in `wire`'s `is_cs_value_type`.

**A measurement in this issue was wrong, and the fix disproved it.** Before implementing, the
class-backed *composite* population was reported empty: a truncated search over
`^public partial class X$` returned fifteen hits, all unions and delegates. That was an artefact
of the truncation. `Layer1String` and `Layer2String` carry `Utf8String`, so their managed
conversion is `Into` and `struct_class` emits them as classes; they appear as `Layer3String`'s
payloads and are now guarded. The snapshot moved, which is how the error surfaced.

**The delegate arm was left byte-for-byte identical**, still
`TypeKind::Delegate(d) if d.kind == DelegateKind::Class` rather than `struct_class::is_class`.
Those are *different predicates*, and swapping them would have changed delegate behaviour as a
side effect of a classification cleanup.



Adjacent, unargued — split out, still open

Whether `?? default` is right even for the types it already covers. A zeroed `Unmanaged` for a
class delegate is a **null function pointer**; Rust invoking it is no better than reading a
fabricated variant, which is the reasoning that decided open item 1 the other way. The file
asserts the policy in a doc-comment and never argues it.

Not folded into `c56f941`: it is a behaviour change to working code, with eight delegate fields
emitting `?? default` in the reference project alone. **Nobody has run it** — what a null callback
actually does at the boundary is unmeasured. So the next step is the same as open item 1's was:
measure first, then decide. `nullable.rs`'s `SubstituteDefault` variant carries a pointer here.



## Enum emission is gated on TypeKind::DataEnum in ten output passes, not on managed_conversion

```issue
id: 5d1ae4c7
kind: bug
severity: medium
status: open
```

### What this traces

`79be256e` names `managed_conversion` as the single lever for reclaiming unit-only enum
machinery — *"Classify a unit-only `DataEnum` as `AsIs` and the downstream falls out"* — while
also recording that something else gates the `Unmanaged` mirror and marshaller and **has not been
traced**. Traced here. The lever hypothesis is false, and the real structure is different in a way
that changes the cost of every option in that issue.

### The processing gate

Every enum output pass opens with the same three-arm match on `TypeKind`, with no reference to
`managed_conversion`, `struct_class`, or variant shape:

```rust
let data_enum = match type_kind {
    TypeKind::DataEnum(e) => e,
    TypeKind::TypePattern(TypePattern::Result(_, _, e)) => e,
    TypeKind::TypePattern(TypePattern::Option(_, e)) => e,
    _ => continue,
};
```

Ten sites, in two forms. Eight bind the payload: `definition.rs`, `body_unmanaged.rs`,
`body_unmanaged_variant.rs`, `body_to_unmanaged.rs`, `body_as_unmanaged.rs`, `body_ctors.rs`,
`body_exception_for_variant.rs`, `body_tostring.rs`. Two match for effect only and discard it:
`all.rs` and `body.rs`. `all.rs`'s form binds `e` in two arms without using it.

`body_from_call.rs` is **not** an eleventh copy — traced. It gates on
`TypeKind::TypePattern(TypePattern::Result(ok, err, _))` with `_ => continue`: no `DataEnum` arm,
no `Option` arm, and it discards the `DataEnum` in the arm it does match. `FromCall` is a `Result`
factory, so the narrower predicate is correct there. **Exclude it from any consolidation.**

### A helper already exists, one layer up

`union_names.rs` declares `data_enum(kind) -> Option<&DataEnum>` and `data_enum_mut`, performing
exactly this match, with a doc-comment explaining the three routes a `DataEnum` takes into the
model. The ten output sites re-implement it inline rather than calling it.

Same shape as `31248473` and `c33b9cf5`: a question one place owns, answered independently
elsewhere. It matters here because any per-enum eligibility rule wants one seam, and there are
currently ten.

### The emission gate that does exist — and it is a single site

`body.rs` computes two predicates the other passes do not:

```rust
let has_wire_only_payload = /* any variant payload is TypeKind::WireOnly */;
let is_managed_only = has_wire_only_payload
    || /* Result/Option whose Ok side is a Service */;
```

with the comment that a `DataEnum` carrying a `WireOnly` payload *"has no FFI-safe Unmanaged form
— it only flows through `Wire<T>`"*.

So a per-enum **"this enum has no `Unmanaged` representation"** concept already exists, at one
site, and is not `managed_conversion`. That is why unit-only enums are already `AsIs`-or-`To` yet
still receive the full machinery — the classification was never load-bearing for emission.

**Where the suppression actually happens — traced.** `is_managed_only` never reaches the
sub-passes, and `body.rs` does not filter on it either. It is inserted once into the template
context and consumed only by three guards in `templates/common/types/enums/body.cs`:

- `{%- if not is_managed_only -%}` around the `[NativeMarshalling(typeof(MarshallerMeta))]`
  attribute;
- `{%- if not is_managed_only %}` around the unmanaged-variants loop and the `Unmanaged` mirror;
- `{%- if not is_managed_only %}` around `[CustomMarshaller]`, `MarshallerMeta` and the
  `Marshaller` ref struct.

So the architecture is: the sub-passes run unconditionally and render their fragments, `body.rs`
pulls them with `.get(type_id).map_or("", ...)` and passes them all into the context, and **the
template declines to interpolate them.** Suppression is a template conditional, one layer further
out than "the assembling pass decides".

Two consequences. A third category could be added as another flag in the same context, guarding
the same three regions, **without touching the ten gate sites at all** — materially cheaper than a
model-pass route. And note that the flag already covers `[NativeMarshalling]`, not just the body,
which is relevant to a plain-enum projection that needs no attribute either. Against that:
suppression at template level means the fragments are still rendered and discarded, which is
harmless for today's managed-only set and worth re-checking before extending the pattern to every
unit-only enum.

### Consequence for 79be256e

Reclassifying a unit-only `DataEnum` as `AsIs` would move struct-vs-class (`definition.rs` reads
`struct_class.is_struct`) and how containing types convert. It would **not** stop
`body_unmanaged.rs`, `body_unmanaged_variant.rs`, `body_to_unmanaged.rs` or `body_as_unmanaged.rs`
from firing, because none of them consults it. `79be256e`'s "The lever" section should be read as
disproved, not as a plan.

### What a unit-only projection would actually need

`is_managed_only` is the wrong predicate to extend. Managed-only means *never crosses FFI*. A
unit-only enum projected as a plain C# `enum` does cross FFI — it is blittable and needs no mirror
*because* it is blittable, which is the opposite reason.

That implies a third category alongside "full machinery" and "managed-only". Naming it, and
deciding whether it belongs in the model or beside `is_managed_only` in `body.rs`, is not this
issue's call and is deliberately not proposed here.

### Measured, and not measured

Static read of the pass sources and the `body.cs` template. **No build, no generation, no
snapshot.**

Both items this issue originally left open have since been traced and are recorded above:
`body_from_call.rs` is `Result`-only, and `is_managed_only` suppresses in the template rather than
in `body.rs` or the sub-passes. An earlier revision of this issue inferred the latter from the
`.get(type_id).map_or("", ...)` pulls and placed it one layer too far in; the template guards are
the actual mechanism.

Still not confirmed by running the generator: that the three template guards are the *only*
consumers of the flag in the rendered output, and what the sub-passes cost when their fragments
are rendered and discarded.

## `_hasValue` was emitted, never written, never read — 19 CS0169 with nothing to surface them

```issue
id: 8f4c1e2a
kind: bug
severity: medium
status: closed
```

**Closed by item 3c (`bdd13b53`).** A forced rebuild of full-scale generated output now reports
**0 warnings**, down from 19.

> **Three claims in the original filing were wrong and are corrected below.** They were written
> before the investigation finished and disproved later in the same session. The measurement held;
> the diagnosis did not.

### Measured

A forced rebuild of `crates/backend_csharp/benches/dotnet` on 2026-08-26:

```
dotnet build -t:Rebuild -v:n
    19 Warning(s)
```

**Every one was CS0169, and every one was `_hasValue`.** No other warning of any kind in the
entire generated surface — the warning profile was one defect repeated, not a long tail.

### The defect

`_hasValue` was emitted by `templates/common/types/enums/definition.cs` and touched nowhere else
in the generator. Nothing wrote it, nothing read it.

`docs/csharp-unions.md` described item 3c as consuming it and said *"the storage exists; what is
missing is the reads"* — but the writes were missing too, so a `Value` consulting the flag would
have returned `null` for every struct-backed union. That is why 3c could not be split into a
writes step and a reads step: writes alone turn CS0169 into CS0414, still a warning.

### Correction 1 — the feedback loop was not severed

The original filing said *"Two projects exist to compile generated output. Neither currently
does."* **That is false.**

The twelve plugin fixtures compile generated C# on **every** `cargo test`, through
`define_plugin!` → `prepare_plugin` → `dotnet build`. They were passing throughout. The loop is
complete; the signal is discarded at one specific point — `build_and_stage` shells out and checks
only the **exit status**:

```rust
let status = Command::new("dotnet").args(["build", "-c", "Release", "-v", "q"]).arg(&csproj).status()?;
if !status.success() { ... }
```

Warnings do not affect exit status, and `-v q` hides them. That is the whole failure, and it is
why `TreatWarningsAsErrors` is the right lever: it converts the warning into a non-zero exit that
this existing check already catches, with no change to the harness.

`backend_plugins/exceptions.dll` is the proof case — its `Try<u32>` becomes a struct-backed
`ResultUintDotnetException` carrying `_hasValue`, compiled green on every run.

### Correction 2 — the output is regenerated, not committed

The original filing called `benches/dotnet` *"committed, full-scale generated output"*. **It is
gitignored.** `tests/reference_project/mod.rs` writes both output directories on every run:

```rust
multibuf.write_buffers_to("tests/reference_project/Bindings")?;
multibuf.write_buffers_to("benches/dotnet")?;
```

So generation is wired to two directories and **nothing compiles either one** — a sharper defect
than "the loop is broken". The 19 CS0169 were measured against fresh output, so the count stands.

### Correction 3 — `benches/dotnet` is drifted, not structurally broken

It fails at `Benchmark.cs(27)` with `CS0117: 'ServiceAsyncBasic' does not contain a definition for
'Create'` — the generator emits `Simple()`. A second stale site follows at line 110 (`Call`).

`Benchmark.cs` has not changed since `9ebfc92`, **2026-04-14**, while the generator kept moving.
A consumer nobody builds, drifting for four months. Not a generator regression, and fixing it is
its own task.

`tests/reference_project/Bindings` fails earlier still, at **restore**: `NU1100` on
`Microsoft.NET.ILLink.Tasks`, pulled in by `IsAotCompatible`, with PackageSourceMapping excluding
both configured sources. A build there reports `0 Warning(s)` — meaningless, since nothing
compiled. That is an environment and package-source problem, and it belongs to item 5.

### What actually fixed it

Item 3c, in one change:

- `definition.cs` emits `_hasValue` only for union-projected enums, so six unit-only enums lost a
  field nothing would ever read.
- Factories and `Unmanaged.ToManaged` write it.
- `HasValue`, `Value` and `TryGetValue` read it.

### Still open: enforcement

The fix removed today's 19. It does not stop the next one. `WarningsAsErrors` on the projects that
compile generated output — the plugin fixtures — would make "field emitted with no reads"
unlandable rather than discovered five items later.

Scope honestly: the fixtures are all **plugin-mode** (`DotnetLibrary`). Library-mode output
(`RustLibrary` → `Bindings/`, `benches/dotnet/`) is what real consumers use, and neither of those
compiles. Shared passes like `definition.cs` are covered either way, which is why this defect was
catchable — but anything emitted only in library mode stays unguarded until item 5.
## AsUnmanaged has no entry point in either generated pipeline, so its empty-state guard cannot fire

```issue
id: 5e2a319c
kind: bug
severity: medium
status: open
```

`AsUnmanaged()` is emitted for composites and for struct-backed unions, and for the latter it carries the empty-state guard from `templates/common/types/enums/body_as_unmanaged.cs`. Nothing calls it. There are two parallel conversion cascades and only one of them is rooted.

### The two cascades

Marshallers root the **owning** conversion, by two different names depending on backing:

```csharp
public Unmanaged ToUnmanaged() { return _managed.ToUnmanaged(); }    // struct-backed, Interop.cs:9949
public Unmanaged ToUnmanaged() { return _managed.IntoUnmanaged(); }  // class-backed,  Interop.cs:3070
```

A composite's owning conversion then calls its fields' owning conversions — `Layer2String.IntoUnmanaged()` calls `the_enum.ToUnmanaged()` (Interop.cs:11159). A composite's `AsUnmanaged()` calls its fields' `AsUnmanaged()` (11173). The second cascade is complete, self-consistent, and entered from nowhere.

### Evidence

Semantic caller queries (Roslyn, not text search), against `tests/reference_project/Bindings/Interop.cs`:

| Method | Callers |
|---|---|
| `ResultUintError.ToUnmanaged()` | `Marshaller.ToUnmanaged()` |
| `Layer3String.IntoUnmanaged()` | `Marshaller.ToUnmanaged()` |
| `ResultUintError.AsUnmanaged()` | none |
| `ResultOptionEnumPayloadError.AsUnmanaged()` | none |
| `Layer3String.AsUnmanaged()` | none |
| `OptionEnumPayload.AsUnmanaged()` | `ResultOptionEnumPayloadError.AsUnmanaged()` |
| `Layer2String.AsUnmanaged()` | `Layer3String.AsUnmanaged()` |

Plus, across `tests/reference_plugins/**` excluding `bin`/`obj`, `\.AsUnmanaged\(\)` returns 6 matches with `truncated: false`, every one inside another `AsUnmanaged`. The `DotnetLibrary` (plugin) pipeline has the same shape as `RustLibrary`, so this is not a forward-interop-only artefact.

### Why this is not a missing reference-project fixture

The obvious remedy is a composite carrying a union-typed field, so the composite's conversion reaches the union's `AsUnmanaged`. That fixture already exists: `Layer2String` has `the_enum` and its `AsUnmanaged` calls `the_enum.AsUnmanaged()`. Still unreachable, because nothing calls `Layer2String.AsUnmanaged()` either. Adding another composite would add unreachable code, not coverage.

### Consequences

- The empty-state guard in `enums/body_as_unmanaged.cs` cannot fire and no test can make it fire. `docs/csharp-unions.md` §413 specifies the contract as `ToUnmanaged()` / `AsUnmanaged()` → throws; only the first is reachable.
- The `2e172709` CHANGELOG entry documents a breaking guard on both methods. Only the `ToUnmanaged` half can affect a consumer.
- Item 4's note "No test asserts that the guard fires — that is 5d" is satisfiable only for `ToUnmanaged`. `Test.Pattern.Union.cs` does that half.

### Open question, not a proposed fix

Whether `AsUnmanaged` is the borrow-side conversion for a marshaller mode that is not currently emitted (`in`/`ref` parameters, pinned slice elements), or whether it is vestigial and should be removed along with its guard. `unmanaged_conversion.rs:78` maps both `ManagedConversion::To` and `::Into` to `.AsUnmanaged()`, which suggests the former. This needs the emitter author, not another search.

### Correction to `docs/csharp-unions.md` Open item 1

Open item 1 states that a class-backed union stored as a composite field "gets an unguarded `.AsUnmanaged()`, so the NRE fires inside the enclosing composite's conversion, not in a marshaller."

The mechanism is right and the measurement is not in doubt — but the method named is the dead one. In the live path the composite calls the field's `.ToUnmanaged()` / `.IntoUnmanaged()`. The NRE fires there.

**This makes Open item 1 more urgent, not less.** The item itself cites both passes emitting the `?... ?? default` form — `body_as_unmanaged.rs:47` *and* `body_to_unmanaged.rs:48` — so the nullability gap is identical in the cascade that actually executes. A reader who checks only the named method finds unreachable code and may conclude the defect is theoretical. It is not.

An earlier revision of this issue claimed the NRE "cannot have been observed through this path" and suggested it may have been derived rather than run. That was wrong: it was found by searching only for `_managed.AsUnmanaged()`, which misses `_managed.IntoUnmanaged()`, the class-backed root. The negative was real but partial.
## Custom-marshalled types are unusable in a consumer's own LibraryImport because Unmanaged and Marshaller are internal

```issue
id: e5f03dbe
kind: bug
severity: medium
status: open
```

`Unmanaged` and `Marshaller` are emitted `internal`. The `LibraryImport` source generator emits code that references both directly, so it can only wire a custom-marshalled type from inside the assembly that declares it. A consumer who puts generated bindings in one project and writes their own `LibraryImport` in another cannot use any custom-marshalled type in their own P/Invoke signatures.

### Measured 2026-08-28

The same declaration, differing only in which project holds it:

    [LibraryImport("reference_project", EntryPoint = "reference_malformed_result_tag")]
    public static partial ResultUintError MalformedResultTag();

- In `Bindings` (the assembly that declares `ResultUintError`) — compiles, runs, marshals correctly.
- In `Tests` (a referencing assembly) — fails.

The failure presents two different ways depending on an unrelated attribute, which is what makes it hard to diagnose:

- Without `[assembly: DisableRuntimeMarshalling]`: **SYSLIB1051**, "Runtime marshalling must be disabled in this project ... to enable marshalling this type." The generator silently falls back to runtime marshalling and reports the fallback, not the cause.
- With it: the fallback is refused and the real cause surfaces as five **CS0122** in `LibraryImports.g.cs` — `ResultUintError.Unmanaged`, `.Marshaller`, `.Marshaller.FromUnmanaged`, `.Marshaller.ToManaged`, `.Marshaller.Free`, each "inaccessible due to its protection level".

Neither message names accessibility as the problem unless the attribute is set, and the attribute is unrelated to the actual defect.

### What this is not

Return-by-value of a union-projected enum is **not** the trigger. `Interop.cs:717-719` declares

    [LibraryImport(NativeLib, EntryPoint = "pattern_result_1")]
    public static partial ResultUintError pattern_result_1(ResultUintError x);

which returns the same custom-marshalled type by value, from the declaring assembly, and has always compiled. An earlier reading of this defect blamed return-by-value and then blamed cross-assembly `[NativeMarshalling]` resolution; both were wrong. The variable is accessibility of the generated helper types.

### Scope

Affects any consumer whose call sites live outside the bindings project — a normal layout, not an exotic one. It does not affect calls made *through* the generated `Interop` class, which is how every existing consumer and every test in this repo works; that is why it has gone unnoticed.

### Options, none taken

- Emit `Unmanaged` and `Marshaller` as `public`. Straightforward, and enlarges the public API surface with types whose members already carry `[Obsolete("intended for use by generated code only")]` and `[EditorBrowsable(Never)]`.
- Emit `[assembly: InternalsVisibleTo(...)]`. Needs a consumer assembly name the generator has no way to know.
- Document the constraint and leave it. Cheapest, and consumers hit SYSLIB1051 with no path to the cause.

### Related, and separate

`DisableRuntimeMarshalling` cannot currently be set on the Bindings assembly at all: it emits `[MarshalAs(UnmanagedType.LPStr)] string` parameters and returns — at least ten sites, result set truncated — which the attribute forbids. That blocks the attribute for AOT reasons independently of this issue, and would need the ASCII-pointer and CStr paths moved to source-generated marshallers first. `Bindings.csproj` already sets `IsAotCompatible` and `Tests.csproj` sets `PublishAot` for Release, so the AOT story otherwise appears to hold; no concrete AOT or trimming failure has been observed, and none of that work should start without one.

### Worked around in-repo

`crates/backend_csharp/tests/reference_project/Bindings/MalformedFixture.cs` — a hand-written, non-generated file placed in the Bindings project precisely because it cannot live in Tests. It exists for item 5f (`1e14d2c6`), which asserts the invalid-tag arm of `Unmanaged.ToManaged()`. `Bindings/` gitignores only `Interop*.cs`, so a hand-written file there is tracked normally.