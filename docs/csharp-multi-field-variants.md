---
chat_url: 'https://claude.ai/chat/dbb65026-426d-4a54-89ee-4cfad6ee63d1'
---

# Multi-field and named enum variants

**Status: proposed. Nothing implemented.**

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

`body_exception_for_variant.cs` emits `return new EnumException<{{ v.type }}>(_{{ v.name }});`. For N fields there is no single `T`. The natural resolution is to construct the case type and use `EnumException<{{ v.case_type }}>`, which is uniform across arities including the current single-payload one — but it changes the generic argument on an existing public member for every payload-carrying variant, so it is a breaking change to consumers that name it. Decide before step 2; do not discover it in step 5.

### Named variants introduce a field-name authority

`union_names::family(&stem)` produces exactly one `field` and one `unmanaged` per variant. Multi-field needs N of each, which is mechanical. The named form additionally needs the *field names themselves* allocated and collision-resolved:

- against `RESERVED` — a field named `Value`, `HasValue`, `Unmanaged`, `Dispose` and so on;
- against sibling members of the enclosing union;
- against each other after any casing transformation.

`proc_macros_impl/src/types/validation.rs:47-53` already rejects struct fields whose name is in `FORBIDDEN_NAMES` (136 entries, gated by the compile-fail test `tests/ui/proc/ty/forbidden_field.rs`), and `:57-59` does the same for variant names. That covers C# keywords but not collisions with generated members, which is what `union_names` exists for.

This is the same defect family as `Issues.md` `4e9a17c3`, `7c8cb22e`, `31248473` and `c33b9cf5` — a naming question answered in more than one place. Design it into `union_names` deliberately rather than deriving names at an emission site.

## Staging

Additive migration. The reference-project snapshot is the gate for steps 1-4: output must be byte-identical, because nothing about the emitted C# changes until step 5.

**Step 1 — widen the model, populate both.** Add the N-payload representation to `core`'s `VariantKind` and to `backend_csharp`'s `Variant` alongside the existing single-payload field. Populate both from the current single payload. No consumer reads the new field yet. Snapshot unchanged.

**Step 2 — migrate `backend_csharp` output passes.** One pass at a time to read the N-payload field, still length 1. Ten passes: `definition`, `body_ctors`, `body_unmanaged`, `body_unmanaged_variant`, `body_to_unmanaged`, `body_as_unmanaged`, `body_case_types`, `body_union_members`, `body_tostring`, `body_exception_for_variant`. Templates gain nested loops that iterate exactly once. Snapshot unchanged. Settle the `EnumException<T>` decision here.

**Step 3 — migrate `union_names`.** `field` and `unmanaged` become per-field collections, still length 1. Extend `every_emitted_name_is_unique` to walk them. Snapshot unchanged.

**Step 4 — migrate `wireio` and `wire`.** `write` / `read` / `live_size` destructure N bindings; `wire/mod.rs`'s three `VariantKind` matches carry N. Still N=1. Snapshot unchanged.

**Step 5 — lift both rejections.** Accept `Fields::Unnamed(n)` and `Fields::Named` in `model.rs`, allocate field names through `union_names`, drop the old single-payload field. Add reference-project fixtures for both forms and accept the snapshot. This is the only step that changes output.

Steps 1-4 are refactors with a byte-identical snapshot as the safety net, and any of them can be abandoned without leaving a broken tree. Step 5 is the only one that needs review of generated C#.

## Not doing

- **Splitting the two rejections.** Shipping `Unnamed(n)` without `Named` leaves a generator that still refuses `enum E { A { x: u32 } }`. They are one gap.
- **Reviving `_old/backend_c` or `_old/backend_cpython`.** They do not compile against today's core and use a different `VariantKind` shape.
- **Changing the `Value` contract.** It is `object?` over case types and is already correct for any arity.

## Unverified

- No build, no generation, no snapshot run. This is a static read of `core/src/lang/types/enums.rs`, `proc_macros_impl/src/types/{model,emit,wireio,validation}.rs`, `backend_csharp`'s enum model and output passes, and all eight templates under `templates/common/types/enums/`.
- The per-pass line counts for step 2 are not measured; the pass list is complete but the size of each edit is not known.
- Whether any consumer outside this repository names `EnumException<T>` on a generated enum, which decides how breaking that half of step 2 is.
