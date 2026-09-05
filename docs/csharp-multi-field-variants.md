---
chat_url: 'https://claude.ai/chat/dbb65026-426d-4a54-89ee-4cfad6ee63d1'
---

# Multi-field and named enum variants

**Status: Steps 1–5 implemented.**

## Objective

The two former parser rejections are removed. Ordinary tuple and named Rust variants now cross the boundary:

```rust
enum E { A(u32, u32) }
enum E { A { x: u32 } }
```

Both forms are covered together by native, generated C#, and wire roundtrips. The implementation follows the supplied `D:\repos\Unions\src\Unions\UnionSpecification.md`: generated cases are the values of the union; `HasValue`, `Value`, and `TryGetValue` agree, and case patterns are exhaustive.

## Why this is a generator gap

Interoptopus is a general Rust-to-C# binding generator. The former one-payload ceiling limited what any consumer could express. Wrapping several fields in a user-defined struct was a workaround, but ordinary Rust variants should not require it. Serialized fields have the same ordered, padding-free wire representation. Native layout is a separate concern: the per-variant sequential helper includes the discriminant before all fields; introducing an extra nested payload struct can change alignment and offsets. The mixed-alignment fixture verifies the actual Rust layout.

The old parser rejected both forms cleanly. Step 5 accepts them only after storage, naming, generated methods, ownership conversions and wire traversal handle every field.

## Where the limit lived

`crates/core/src/lang/types/enums.rs:12`:

```rust
pub enum VariantKind {
    Unit,
    Tuple(TypeId),   // exactly one
}
```

Core is backend-agnostic. `_old/backend_c` and `_old/backend_cpython` emit `// TODO - OMITTED DATA VARIANT - BINDINGS ARE BROKEN` for a typed variant at all, which is the historical reason the model normalised to one payload: C's tagged-union payload arm is a struct. Those crates are dead and do not compile against today's core, so there is exactly one live backend to keep in step.

The parser formerly enforced the limit and `emit.rs` mapped one type per variant. Storage is now `VariantKind::Tuple(Vec<TypeId>)` or `VariantKind::Struct(Vec<Field>)`; backend variants store one resolved field collection. The derived `payloads()` views preserve declaration order and names without duplicating payload state.

## Existing output contracts retained

Read of all eight templates under `templates/common/types/enums/` plus `body_union_members.rs`:

- **`Value` does not see payload arity.** `body_union_members.cs` emits `public object? Value => ... _variant switch { {{ v.tag }} => new {{ v.case_type }}(...) }`. It returns the boxed *case type*, not the payload. The 3c contract is unaffected by widening.
- **Case types already take N positional parameters.** `body_case_types.cs` emits a `readonly record struct`. `record struct C(A a, B b, C c)` is legal and deconstructs in patterns, so the consumer-facing shape needs no new concept.
- **A per-variant unmanaged struct already exists.** `body_unmanaged_variant.cs` emits `[StructLayout(LayoutKind.Sequential)] internal unsafe struct Unmanaged{{ variant }} { internal {{ discriminant_type }} _variant; internal {{ unmanaged_name }} _{{ variant }}; }`. Widening means that struct gains fields; the `[FieldOffset(0)]` overlay in `body_unmanaged.cs` is unchanged.
- **`body_tostring.cs`** prints `Name(...)` for any payload-carrying variant and needs nothing.
- **`body_from_call.cs`** gates on `TypePattern::Result` only and is out of scope entirely.
- **`union_names` already allocates and collision-resolves per-variant names** — `stem`, `factory`, `is_check`, `accessor`, `field`, `unmanaged`, `case_type`, with `every_emitted_name_is_unique` as the invariant. The machinery exists; it is per-variant rather than per-field.

## Migration sites

The table records the original single-payload implementation and the completed migration scope.

| Site | Before | Migration |
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

The two public C# contract decisions are resolved additively below.

### `EnumException<T>` takes one type parameter

`body_exception_for_variant.cs` emits `return new EnumException<{{ v.type }}>(_{{ v.name }});`. For N fields there is no single `T`. The natural resolution is to construct the case type and use `EnumException<{{ v.case_type }}>`, which is uniform across arities including the current single-payload one — but it changes the generic argument on an existing public member for every payload-carrying variant, so it is a breaking change to consumers that name it. Preparatory Steps 2–4 preserve the existing `EnumException<TPayload>` behavior and byte-identical output. Step 5 selects the additive contract below.

- [x] Selected additive exception contract: preserve `EnumException<TPayload>` for single-payload variants and use `EnumException<TCase>` for new multi-field variants, using their existing generated case types. No new exception type is needed.

- [x] Selected additive `AsVariant()` contract: single-payload accessors retain their payload return type; multi-field accessors return the existing generated case type. Its positional properties also supply wire access to each field. The preparatory arity guards are removed after implementing this path.

### Named variants introduce a field-name authority

`union_names` allocates both field-name scopes. Backing members remain `_Stem` for one payload and become `_Stem_0`, `_Stem_1`, etc. for multiple payloads. The unmanaged helper and its offset-zero overlay remain one per variant, with these members inside the sequential helper.

Case properties retain `Value` for existing one-slot tuple cases. New tuple cases use `Item1` through `ItemN`; named cases preserve declared spelling (without a Rust raw-identifier prefix). Collisions receive `Field`, then numbered suffixes. The allocator checks:

- the union's reserved names and every sibling generated member;
- the enclosing case type and record members such as `Equals`, `GetHashCode`, and `Deconstruct`;
- previously allocated properties in the same case.

Factories, case constructors, accessors, exception creation, and wire serialization/size use those allocated names. Both outer-member and case-property uniqueness invariants are tested. Different cases may reuse a property name.

`proc_macros_impl/src/types/validation.rs:47-53` already rejects struct fields whose name is in `FORBIDDEN_NAMES` (136 entries, gated by the compile-fail test `tests/ui/proc/ty/forbidden_field.rs`), and the same policy applies to variant names and named variant fields. That covers C# keywords but not collisions with generated members, which is what `union_names` exists for.

This follows the same naming authority required by `Issues.md` `4e9a17c3`, `7c8cb22e`, `31248473` and `c33b9cf5` — a naming question answered in more than one place. No emission site independently sanitizes a field name.

## Staging

Additive migration. The reference-project snapshot is the gate for steps 1-4: output must be byte-identical, because nothing about the emitted C# changes until step 5.

- [x] **Step 1 — open the N-payload seam.** Add a `Payload<'a> { ty, name }` view type and a `Variant::payloads()` accessor to both `core` and `backend_csharp`, derived from the existing single-payload storage. Step 2 now consumes this view; the single-payload storage is unchanged.

Deliberately *not* done as originally written here, which called for a second stored field populated alongside the first. Duplicated stored state leaks into `PartialEq`, `Hash`, serde and every construction site, and the two copies drift. A derived accessor gives step 2 the same migration surface — one place to widen — at none of that cost. When step 5 replaces the storage, `payloads()` changes behind its callers rather than being reconciled with them.

`backend_csharp`'s payloads are *resolved*, not declared: `Result<(), ()>` declares `Ok(T)` but resolves `()` to no C# payload, so it yields no slots while `can_carry_payload` stays true. Union eligibility asks the latter. Step 2 migrates passes that depend on this distinction, so preserve it rather than collapsing the two questions.

- [x] **Step 2 — migrate `backend_csharp` output passes.** The output passes read the N-payload view, still at most length 1. Eleven output passes: `definition`, `body_ctors`, `body_unmanaged`, `body_unmanaged_variant`, `body_to_unmanaged`, `body_as_unmanaged`, `body_case_types`, `body_union_members`, `body_tostring`, `body_exception_for_variant`, and `body` (disposal). Migrate the payload-dependent model decisions in `managed_conversion` and `projection` as well. Templates now iterate the payload collection; existing nonempty collections have exactly one slot. Snapshot unchanged; preserve the existing single-payload exception contract.

- [x] **Step 3 — migrate `union_names`.** `field` is a payload-member collection, still length 1. `payload_fields()` pairs resolved payload types with centrally allocated member names for nine output consumers and eight templates. `every_emitted_name_is_unique` walks the whole collection, including sibling and reserved-name checks. The per-variant `UnmanagedVariant` helper/overlay remains unchanged; its members use the same field collection. Historical unit-variant reservations are preserved. Snapshot unchanged.

- [x] **Step 4 — migrate `wireio` and `wire`.** The proc-macro model exposes a derived payload iterator; WireIO bounds and `write` / `read` / `live_size` iterate bindings in field order. C# wire serialize / deserialize / size consume `Variant::payloads()` instead of matching single-payload storage. Still N<=1. The serializer and sizer explicitly guard the pending multi-field accessor contract. Snapshot unchanged.

- [x] **Step 5 — lift both rejections.** Accepted `Fields::Unnamed(n)` and `Fields::Named`, including empty tuple/named shapes; widened core and backend storage; allocated per-field names; emitted both selected contracts. Every bound, dependency registration, conversion and wire operation visits all fields. Reference fixtures cover mixed alignment, all payload shapes, collisions, managed-only strings/vectors and owned strings. The reviewed snapshot adds only the new fixture output plus the inventory hash update; existing single-payload C# contracts remain unchanged.

Steps 1-4 are refactors with a byte-identical snapshot as the safety net, and any of them can be abandoned without leaving a broken tree. Step 5 is the only one that needs review of generated C#.

## Not doing

- **Splitting the two rejections.** Shipping `Unnamed(n)` without `Named` leaves a generator that still refuses `enum E { A { x: u32 } }`. They are one gap.
- **Reviving `_old/backend_c` or `_old/backend_cpython`.** They do not compile against today's core and use a different `VariantKind` shape.
- **Changing the `Value` contract.** It is `object?` over case types and is already correct for any arity.

## Validation

- [x] Step 2 reference output is byte-identical: `reference_project::interop` passed on 2026-09-05 without changing snapshots.
- [x] The regenerated bindings compiled and their C# suite passed through `reference_project::csharp_suite`. The project targets `net11.0` with inherited `LangVersion=preview`. The focused nextest run passed 2 tests; 85 unrelated tests were filtered. At Step 2 the harness launched `dotnet run`; that evidence alone did not prove an explicit patched-runtime launch.
- [x] Source search confirms all eleven output consumers and both model consumers use the payload view, directly or through `payload_fields()`; the existing union eligibility rule still reads `can_carry_payload`.
- [x] Steps 3–4 naming checks passed 12/12; existing C# wire checks passed 20/20, including the wire plugin round trip (2026-09-05).
- [x] Rust wire checks passed 23/23, including signed/explicit tags, exact unit and tuple payload bytes, live-size agreement, unknown tags and truncated input.
- [x] Steps 3–4 reference generation and C# execution passed 2/2; the committed reference snapshot remains byte-identical. Exact commit validation re-runs the reference and core wire gates.


- [x] Step 5 metadata/order/registration, later-field safety flags, generic payload bounds, and native alignment checks passed. The generic fixture also covers an existing `where` clause without a trailing comma.
- [x] Step 5 Rust wire checks passed 25/25, including mixed-width signed tags, every truncated prefix, empty payload shapes, and owned UTF-8 interoperability with the standard string format.
- [x] Step 5 generated C# passed 237/237 xUnit tests, including 14 new facts, on 2026-09-05. Runtime operation `1d4ce8b9413a3aee3babbda5b08c88b9` launched `D:\repos\runtime-async-dynamicmethod\artifacts\tests\coreclr\windows.x64.Release\Tests\Core_Root\corerun.exe` explicitly and reported .NET 11.0.0-dev, win-x64; the build used SDK `11.0.100-preview.7.26381.103`.
- [x] Patched-runtime selection is enforced by the reference harness and `TestHostRuntime=PatchedCoreRunRequired`. `CSHARPMPC_PATCHED_CORERUN` may select a host; otherwise the bounded adjacent runtime checkout is resolved. An invalid explicit path or missing patched artifacts fails, with no stock-runtime fallback.
- [x] C# checks cover exhaustive case patterns, agreement of `Value`/`HasValue`/`TryGetValue`, default/null guards, single-payload exception compatibility, repeated borrows, active-field move/disposal, and raw/managed/owned wire roundtrips. Sixteen malformed later-field reads return native allocation counts to the pre-read baseline.
- [x] The reference snapshot was reviewed and accepted: all 29,197 original lines remain in order after normalizing the four API-hash occurrences; 1,631 added lines belong to the new fixtures and their functions/wire wrappers. The hash changed from `aa8ee21da66c9c72` to `9a370e2a9fcbf4ec`.

### Owned-string wire completion

The new roundtrip exposed a pre-existing gap: `ffi::String` declared `WIRE_SAFE=true`, but Rust `WireIO` contained `todo!()` and C# had no UTF-8 pattern mapping. These existing methods now implement the standard u32-byte-length/UTF-8 format. C# serialization borrows `Utf8String.String`; deserialization uses `Utf8String.From`, rejects short/invalid UTF-8 input, and tracks each newly allocated native string until the whole value has been constructed. A later-field read failure disposes those allocations; successful reads transfer ownership to the returned value.

### Exact checkpoint gate

Transaction `WSMCP-d06966127c45a25f25d7b93d` records the exact gate operation and integrated commit. Required targets re-run the core tests with `unstable-plugins` enabled and the reference snapshot plus explicitly patched C# suite. The gate also runs workspace Clippy and workspace nextest and requires zero uncovered changes. Package publication is a separate follow-up; this implementation does not change package versions.
