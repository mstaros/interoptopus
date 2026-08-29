# C# call-scoped marshalling for read-only Rust pointers

Status: **implemented.** Landed in `3aeca3c8d5f1f9640dcaecdd8fec2b9832b879d4`. Guarded
validation passed for both Rust and generated C# suites.

This is an optimization and a generated-API change, not a defect fix. The existing `ref`
marshaller moves owned payloads into native storage and writes them back correctly. The change
removes that ownership round-trip only where the Rust model exposes a read-only pointer and
the managed pointee requires custom marshalling.

## Objective

For a Rust parameter such as `x: &UseString`, generate the managed overload as `in UseString`
and select a `MarshalMode.ManagedToUnmanagedIn` marshaller whose `ToUnmanaged` calls
`AsUnmanaged()`. The managed value retains ownership, and no conversion runs after the native
call to write it back.

The raw ABI declaration remains `IntPtr`. Only the managed overload changes.

## Evidence and boundary

The distinction already survives into the backend:

- `#[ffi]` emits the exact parameter type's `TypeInfo::id()`.
- `&T` and `&mut T` have different derived `TypeId`s and become `ReadPointer` and
  `ReadWritePointer` respectively.
- The core intentionally gives `&T` and `*const T` the same derived `TypeId`; likewise,
  `&mut T` and `*mut T` share the read/write pointer identity. The backend therefore knows
  mutability, not Rust source-level provenance.
- The C# model preserves the distinction it does have as `IntPtrHint::Read` and
  `IntPtrHint::ReadWrite`.
- The collapse happens later: pointer overload generation always selects `family.by_ref`,
  whose parameter decorator is always `ref`.

No new borrow flag or inventory entity is needed. Selection derives from the existing pointer
hint. The implementation's honest model boundary is the read-only pointer family, even though
`&T` is the motivating and ordinarily generated shape.

A read-only Rust pointer is not, by itself, proof that the pointer cannot escape the call.
`ref1(x: &i64) -> &i64` returns the input borrow, and its C# test reads the returned pointer.
Changing every `IntPtrHint::Read` overload to `in` would let callers pass a temporary and could
make that returned pointer dangle.

Therefore the optimization is intentionally narrower:

- Use `in` only when the original pointer hint is `Read` **and** the pointee's managed
  conversion requires a custom marshaller.
- Keep direct / `AsIs` read-only pointers as `ref`; this includes `ref1`.
- A custom-marshalled native mirror is already call-scoped under the current `ref` path, so
  retaining its outer pointer after return is not a supported working contract this change
  removes.

## Ownership rules

| Rust-side shape | Managed parameter | Conversion | After-call write-back |
|---|---|---|---|
| owned `T` | `T` | existing moving `ToUnmanaged` / `IntoUnmanaged` path | existing behavior |
| read-only pointer family (`&T` / `*const T`), custom-marshalled pointee | `in T` | `AsUnmanaged()` | none |
| read-only pointer family, direct / `AsIs` pointee | existing `ref T` | existing direct path | unchanged |
| read/write pointer family (`&mut T` / `*mut T`) | `ref T` | existing moving marshaller | required and unchanged |

`pattern_string_6b` assigns a new `UseString` through `&mut`; its C# test verifies the new
`s1` and `s2` values. That makes write-back part of the observable contract.

By-value ownership transfer must also remain unchanged. Rust owns and drops values passed by
value; borrowing that path would leave C# believing it owns memory Rust has freed.

`Slice` and `SliceMut` remain classified with their existing copy/borrow semantics. A
read-only pointer to either may use the new `in` mode, but this change does not alter their
conversion strategy or ownership model.

## Generated marshaller shape

Every applicable generated managed type keeps its existing `MarshalMode.Default` marshaller.
That marshaller remains the sole moving/write-back path for by-value, `ref`, `out`, and return
positions.

The type also emits an internal `InMarshallerMeta` entry point registered for
`MarshalMode.ManagedToUnmanagedIn`, backed by a distinct in-marshaller:

- `FromManaged` captures the managed value for the call;
- `ToUnmanaged` calls the managed type's `AsUnmanaged()` conversion family;
- it has no `ToManaged` / `FromUnmanaged` write-back path;
- cleanup must not free payloads still owned by the managed value.

The in-marshaller is selected at the generated parameter with
`[MarshalUsing(typeof(T.InMarshallerMeta))] in T`. It is deliberately **not** added to the
type's `NativeMarshalling` default entry point: the source generator uses
`ManagedToUnmanagedIn` for both by-value and `in` P/Invoke parameters, so a type-wide
registration would also make owned by-value arguments borrow and would violate Rust ownership.
The existing default entry point therefore remains the only implicit choice.

The exact stateful-marshaller member set follows the source-generator contract and must be
proven by compiling the generated C# under the repository's .NET 11 SDK.

## Enum and union constraints

Composite and enum conversion families must enter `AsUnmanaged` consistently. Enum emission
has additional invariants:

- A struct-backed union has `_hasValue`; its `AsUnmanaged` empty-state guard becomes reachable
  for the first time through the new in-marshaller and needs an execution test.
- A class-backed union has no `_hasValue`; its empty state is a null reference. `in` is a
  readonly reference to that managed reference, so it needs separate compile-and-run coverage.
- Naming is untouched. Any emitted variant-derived member continues to use `Variant::stem`.
- Unit-only plain enums are `AsIs` and therefore stay on the existing `ref` path under the
  scoped eligibility rule.

## Verification

Required text/model checks:

- a shared custom-marshalled parameter renders as `[MarshalUsing(typeof(T.InMarshallerMeta))] in T`;
- a mutable parameter still renders as `ref T`;
- `ref1(ref long)` remains unchanged;
- the generated type registers both the existing default marshaller and the new
  `ManagedToUnmanagedIn` marshaller;
- the in-marshaller calls `AsUnmanaged`, not `ToUnmanaged` or `IntoUnmanaged`.

Required C# execution checks:

- call a shared-borrow function repeatedly with the same owned-payload value, then read and
  dispose that value successfully;
- preserve the existing by-value spent-value behavior;
- preserve `pattern_string_6b` mutable replacement/write-back behavior;
- borrowing a default struct-backed union reaches the `AsUnmanaged` guard and throws the
  expected `InvalidOperationException`; a constructed value does not;
- exercise a class-backed custom-marshalled shared borrow separately;
- preserve the `ref1` returned-pointer test on the direct path.

Validation runs `cargo test -p interoptopus_csharp`; the guarded result must include the Rust
test step and the C# suite step.

### What green verification establishes

- the intended overload and marshaller mode were selected;
- the tested managed values retain observable ownership after a shared-borrow call;
- mutable write-back and by-value transfer remain observable;
- the previously dead enum `AsUnmanaged` guard is reachable.

### What it does not establish

The feature-gated Rust allocator probes described in
[the allocation-observability design](csharp-allocation-observability.md) now exercise direct and
composite shared borrows, mutable string write-back, `ffi::Vec<T>`, Wire, and unique/shared
Rust-created services. That document is the live source for exact coverage, validation results,
and visibility boundaries.

Those gauges remain narrower than general native-memory safety. They do not observe C#
`GCHandle` or `Marshal.AllocHGlobal`, diagnose use-after-free or double-free directly, or cover
callback closure and asynchronous task-handle lifetimes. Equal snapshots establish equal live
Rust-allocator state for the enumerated paths, not absence of balanced allocation/deallocation
events within a call.

## Compatibility

Changing an affected overload from `ref T` to `in T` is a public source break: callers that
write `ref` must update to `in` or use the permitted call form. Add an entry under
`[Unreleased] / ⚠️ Breaking` in `crates/backend_csharp/CHANGELOG.md`. Describe the change as
an ownership-transfer optimization and API change, not as a bug fix.
