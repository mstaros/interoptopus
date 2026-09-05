---
chat_url: 'https://claude.ai/chat/dbb65026-426d-4a54-89ee-4cfad6ee63d1'
---

# Multi-field and named enum variants

**Status: Steps 1–2 implemented; Steps 3–5 pending.**

## Objective

Lift the two rejections at `proc_macros_impl/src/types/model.rs:95-96` so that ordinary Rust enums cross the boundary:

```rust
enum E { A(u32, u32) }        // "Tuple variants with multiple fields are not supported"
enum E { A { x: u32 } }       // "Struct variants are not supported"
```

Both are rejected today. Neither is exotic Rust; the named form is arguably the more common way to write a payload-carrying variant.

## Why this is a generator gap

Interoptopus is a general Rust-to-C# binding generator. The one-payload-per-variant ceiling is a limit on what any consumer can express, not a missing convenience for a particular one. A consumer hitting it can work around it by declaring a struct per variant and carrying that as the single payload — the bytes on the wire are identical either way, because the C-compatible layout of a multi-field variant *is* an anonymous struct. So the question this document answers is not "is the data expressible" but "who authors the struct": the consumer, once per variant, forever, or the generator.

The limit is not a footgun. Both rejections are clean compile errors with plain messages, so no consumer can accidentally ship a broken binding through them. The cost is that the generator turns you away at the door rather than emitting the natural shape.

## Where the limit lives

`crates/core/src/lang/types/enums.rs:12`:

```rust
pub enum VariantKind {
    Unit,
    Tuple(TypeId),   // exactly one
}
```

Core is backend-agnostic. `_old/backend_c` and `_old/backend_cpython` emit `// TODO - OMITTED DATA VARIANT - BINDINGS ARE BROKEN` for a typed variant at all, which is the historical reason the model normalised to one payload: C's tagged-union payload arm is a struct. Those crates are dead and do not compile against today's core, so there is exactly one live backend to keep in step.

`proc_macros_impl/src/types/model.rs:93-96` enforces it at parse time, and `emit.rs:271-276` maps `VariantData` to `VariantKind`.

## Already arity-agnostic — verified, no change needed

Read of all eight templates under `templates/common/types/enums/` plus `body_union_members.rs`:

- **`Value` does not see payload arity.** `body_union_members.cs` emits `public object? Value => ... _variant switch { {{ v.tag }} => new {{ v.case_type }}(...) }`. It returns the boxed *case type*, not the payload. The 3c contract is unaffected by widening.
- **Case types already take N positional parameters.** `body_case_types.cs` emits a `readonly record struct`. `record struct C(A a, B b, C c)` is legal and deconstructs in patterns, so the consumer-facing shape needs no new concept.
- **A per-variant unmanaged struct already exists.** `body_unmanaged_variant.cs` emits `[StructLayout(LayoutKind.Sequential)] internal unsafe struct Unmanaged{{ variant }} { internal {{ discriminant_type }} _variant; internal {{ unmanaged_name }} _{{ variant }}; }`. Widening means that struct gains fields; the `[FieldOffset(0)]` overlay in `body_unmanaged.cs` is unchanged.
- **`body_tostring.cs`** prints `Name(...)` for any payload-carrying variant and needs nothing.
- **`body_from_call.cs`** gates on `TypePattern::Result` only and is out of scope entirely.
- **`union_names` already allocates and collision-resolves per-variant names** — `stem`, `factory`, `is_check`, `accessor`, `field`, `unmanaged`, `case_type`, with `every_emitted_name_is_unique` as the invariant. The machinery exists; it is per-variant rather than per-field.

## Not arity-agnostic — the actual work

| Site | Today | Needs |
|---|---|---|
| `core .../enums.rs:12` | `Tuple(TypeId)` | carry N, and field names for the named form |
| `backend_csharp .../kind/enums.rs:6` | `Variant { ty: Option<TypeId>, .. }` | N payload ids |
| `proc_macros_impl model.rs:93-96` | two rejections | accept `Unnamed(n)` and `Named` |
| `emit.rs` 80, 119, 158 | one type bound per variant for `WIRE_SAFE` / `RAW_SAFE` / `ASYNC_SAFE` | N bounds |
| `emit.rs` 269-276, 410-411 | `VariantKind` construction, payload registration | N |
| `wireio.rs` 206-211, 305-308, 386-394 | destructures `#name::#vname(__inner)` | N bindings, `write` / `read` / `live_size` |
| `enum_variants.rs:45-69` | `VariantKind::Tuple(id)` to one cs id | N |
| `fallback.rs:148-154` | `payload_variant(name, tag, ty)` | synthesised `Result`/`Option` carriers stay single-payload; signature only |
| `wire/mod.rs` 363, 397, 436 | `VariantKind::Tuple(t) => Some(*t)` | N, in serialize / deserialize / size |
| `definition.cs` | `{{ variant.type }} _{{ variant.name }};` | one backing field per payload field |
| `body_ctors.cs` | factory `(T value)`, `As{{name}}()` return, case ctor `value.Value` | N parameters, N reads |
| `body_to_unmanaged.cs`, `body_as_unmanaged.cs` | one assignment per variant | N |
| `body_unmanaged.cs` | `new {{v.case_type}}(_{{v.name}}._{{v.name}}{{v.to_managed}})` | N arguments |
| `body_case_types.cs` | `({{ payload }} Value)` | N named parameters |
| `body_union_members.cs` | `new {{ v.case_type }}(_{{ v.stem }})` | N arguments |

Two of these are decisions rather than mechanical widening, and are called out below.

### `EnumException<T>` takes one type parameter

`body_exception_for_variant.cs` emits `return new EnumException<{{ v.type }}>(_{{ v.name }});`. For N fields there is no single `T`. The natural resolution is to construct the case type and use `EnumException<{{ v.case_type }}>`, which is uniform across arities including the current single-payload one — but it changes the generic argument on an existing public member for every payload-carrying variant, so it is a breaking change to consumers that name it. Preparatory Steps 2–4 preserve the existing `EnumException<TPayload>` behavior and byte-identical output. The multi-field contract must be selected before Step 5.

- [ ] Select the multi-field exception contract: preserve `EnumException<TPayload>` for existing single-payload variants and use `EnumException<TCase>` for new multi-field variants, or use `EnumException<TCase>` uniformly and accept the breaking change for consumers that catch or name the current generic exception. No new exception type is needed.

- [ ] Select the `AsVariant()` return shape for multi-field variants before Step 5. Existing single-payload accessors return their payload type; a generated case type or a C# tuple would express multiple fields. The preparatory constructor and exception passes reject unsupported arities explicitly until these contracts are selected, so widening storage cannot silently discard fields.

### Named variants introduce a field-name authority

`union_names::family(&stem)` produces exactly one `field` and one `unmanaged` per variant. Multi-field needs N of each, which is mechanical. The named form additionally needs the *field names themselves* allocated and collision-resolved:

- against `RESERVED` — a field named `Value`, `HasValue`, `Unmanaged`, `Dispose` and so on;
- against sibling members of the enclosing union;
- against each other after any casing transformation.

`proc_macros_impl/src/types/validation.rs:47-53` already rejects struct fields whose name is in `FORBIDDEN_NAMES` (136 entries, gated by the compile-fail test `tests/ui/proc/ty/forbidden_field.rs`), and `:57-59` does the same for variant names. That covers C# keywords but not collisions with generated members, which is what `union_names` exists for.

This is the same defect family as `Issues.md` `4e9a17c3`, `7c8cb22e`, `31248473` and `c33b9cf5` — a naming question answered in more than one place. Design it into `union_names` deliberately rather than deriving names at an emission site.

## Staging

Additive migration. The reference-project snapshot is the gate for steps 1-4: output must be byte-identical, because nothing about the emitted C# changes until step 5.

- [x] **Step 1 — open the N-payload seam.** Add a `Payload<'a> { ty, name }` view type and a `Variant::payloads()` accessor to both `core` and `backend_csharp`, derived from the existing single-payload storage. Step 2 now consumes this view; the single-payload storage is unchanged.

Deliberately *not* done as originally written here, which called for a second stored field populated alongside the first. Duplicated stored state leaks into `PartialEq`, `Hash`, serde and every construction site, and the two copies drift. A derived accessor gives step 2 the same migration surface — one place to widen — at none of that cost. When step 5 replaces the storage, `payloads()` changes behind its callers rather than being reconciled with them.

`backend_csharp`'s payloads are *resolved*, not declared: `Result<(), ()>` declares `Ok(T)` but resolves `()` to no C# payload, so it yields no slots while `can_carry_payload` stays true. Union eligibility asks the latter. Step 2 migrates passes that depend on this distinction, so preserve it rather than collapsing the two questions.

- [x] **Step 2 — migrate `backend_csharp` output passes.** The output passes read the N-payload view, still at most length 1. Eleven output passes: `definition`, `body_ctors`, `body_unmanaged`, `body_unmanaged_variant`, `body_to_unmanaged`, `body_as_unmanaged`, `body_case_types`, `body_union_members`, `body_tostring`, `body_exception_for_variant`, and `body` (disposal). Migrate the payload-dependent model decisions in `managed_conversion` and `projection` as well. Templates now iterate the payload collection; existing nonempty collections have exactly one slot. Snapshot unchanged; preserve the existing single-payload exception contract.

- [ ] **Step 3 — migrate `union_names`.** `field` and `unmanaged` become per-field collections, still length 1. Extend `every_emitted_name_is_unique` to walk them. Snapshot unchanged.

- [ ] **Step 4 — migrate `wireio` and `wire`.** `write` / `read` / `live_size` destructure N bindings; `wire/mod.rs`'s three `VariantKind` matches carry N. Still N=1. Snapshot unchanged.

- [ ] **Step 5 — lift both rejections.** Accept `Fields::Unnamed(n)` and `Fields::Named` in `model.rs`, allocate field names through `union_names`, and widen the single-payload storage itself — `VariantKind::Tuple(TypeId)` and `Variant::ty`. Because step 1 derived `payloads()` rather than duplicating the field, there is no second copy to reconcile: the accessor starts yielding N slots. Complete the pending per-field names and the accessor/exception contracts, then remove the preparatory arity guards. Add reference-project fixtures for both forms and accept the snapshot. This is the only step that changes output.

Steps 1-4 are refactors with a byte-identical snapshot as the safety net, and any of them can be abandoned without leaving a broken tree. Step 5 is the only one that needs review of generated C#.

## Not doing

- **Splitting the two rejections.** Shipping `Unnamed(n)` without `Named` leaves a generator that still refuses `enum E { A { x: u32 } }`. They are one gap.
- **Reviving `_old/backend_c` or `_old/backend_cpython`.** They do not compile against today's core and use a different `VariantKind` shape.
- **Changing the `Value` contract.** It is `object?` over case types and is already correct for any arity.

## Validation

- [x] Step 2 reference output is byte-identical: `reference_project::interop` passed on 2026-09-05 without changing snapshots.
- [x] The regenerated bindings compiled and their C# suite passed through `reference_project::csharp_suite` on C# 15 / .NET 11 preview 7 with the patched runtime. The focused nextest run passed 2 tests; 85 unrelated tests were filtered.
- [x] Source search confirms all eleven output consumers and both model consumers use `payloads()`; the existing union eligibility rule still reads `can_carry_payload`.

## Unverified

- Steps 3–5 still require their stated validation gates. Named and multi-field Rust variants remain rejected until Step 5.
- Named-field allocation and the per-field naming migration remain Step 3 work.
- Whether any consumer outside this repository names `EnumException<T>` on a generated enum, which determines the consumer impact of a uniform case-type exception contract in Step 5.
