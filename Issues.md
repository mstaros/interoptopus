# Issues

## validate() should reject a Vec pattern used without its builtins_vec registration

```issue
id: 2a6da76a
kind: bug
severity: high
status: open
```

### Symptom

Registering `ffi::Vec<T>` in a function signature without also registering `builtins_vec!(T)` produces C# that references a type the backend never emits. Rust compiles cleanly, `RustLibrary::process()` succeeds, `write_buffers_to` succeeds. The only signal is a C# compile error in generated code:

```
error CS0246: The type or namespace name 'VecByte' could not be found
error CS0246: The type or namespace name 'VecUtf8String' could not be found
```

This is not a one-off. It occurred twice in one session, once per newly introduced element type - first `ffi::Vec<u8>`, then `ffi::Vec<ffi::String>`. Every new element type will hit it again.

### Why validate() is the right place

`validate()` already walks the full inventory and is the only component that sees both the declared surface and the registered builtins. The backend cannot decide at emit time whether a missing builtin is an error or a deliberate omission; the inventory can.

Moving the failure from `csc` to `cargo test` puts it seconds earlier, at the call site, naming the missing macro.

### Proposed fix

Collect every `TypePattern::Vec(T)` reachable from any registered function signature, and diff against registered builtins. Fail with something like:

`ffi::Vec<u8> is used by repo_git_dir but builtins_vec!(u8) is not registered`

Collection must be transitive: `Vec<T>` inside a struct field or enum payload needs the builtin just as much as one in a top-level signature.

`builtins_string!()` has the identical hazard and is simply remembered more often.

### Note

Per CONTRIBUTING, this needs a reference-project test. Since the assertion is that something *fails*, a `trybuild`-style negative case is likely the right shape.
## reference_project snapshot not re-accepted after the AsSpan()/ToArray() template change

```issue
id: ccb105a2
kind: bug
severity: low
status: open
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
## Enum variant discriminant is the declared tag for unit variants but the positional index for payload variants

```issue
id: 09b82d44
kind: bug
severity: high
status: open
```

### Symptom

`crates/backend_csharp/src/pass/model/common/types/kind/enum_variants.rs` derives a variant's
discriminant differently depending on whether the variant carries a payload:

```rust
VariantKind::Unit(tag)           => (*tag, None),
VariantKind::Tuple(rust_type_id) => (index.cast_signed(), Some(cs_type_id)),
```

Unit variants use their declared discriminant. Payload variants use their **positional
index**. For any enum mixing the two with explicit discriminants, the managed `_variant`
field and the native Rust tag disagree.

Example: `enum E { A = -1, B(u32) }` gives `A` the declared `-1` and `B` the positional `1`,
while Rust assigns `B` whatever its own discriminant rules produce.

### Root cause

The information does not exist in the core model. `crates/core/src/lang/types/enums.rs`:

```rust
pub enum VariantKind {
    Unit(isize),   // carries a discriminant
    Tuple(TypeId), // carries none
}
```

`Tuple` has nowhere to put a tag, so the backend has no alternative to the index. This is a
core-model gap, not a backend bug — the backend is doing the only thing it can.

### Impact

Silent at generation time. `_variant` is written into the explicit-layout `Unmanaged` struct
and read back by `Unmanaged.ToManaged()`, so a mismatch corrupts the round trip for affected
enums rather than failing loudly.

No enum in `crates/reference_project/src/types/enums.rs` currently triggers it: `EnumNegative`
uses explicit discriminants but is unit-only, and `EnumPayload` has payloads but no explicit
discriminants. A single reference-project enum combining both would expose it.

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

The alternative — adding a second element to `VariantKind::Tuple` — keeps the discriminant
conditional on variant shape and would need revisiting when named and multi-field variants
are added.

### Blast radius

| Location | Change |
|---|---|
| `core/src/lang/types/enums.rs` | struct, `VariantKind`, `Variant::new` signature |
| `proc_macros_impl` | must compute discriminants (see below) |
| `backend_csharp/.../enum_variants.rs` | read `v.tag` unconditionally |
| `backend_c`, `backend_cpython` | defunct but still compile against these types |
| serde inventory shape | changes under the `serde` feature |

The substantive work is in the proc macro: it must replicate Rust's discriminant assignment —
explicit values where given, previous-plus-one otherwise — across mixed unit and payload
variants. **Unverified:** whether it currently sees explicit discriminants on payload
variants at all. That determines whether this is plumbing or real logic.

### Note

Needs a reference-project enum mixing explicit discriminants with payload variants, plus a
round-trip assertion. This blocks `docs/csharp-unions.md`, whose `ToManaged()` tag validation
would otherwise validate against the wrong tag set — but it is a correctness bug in its own
right and should be reviewed independently of that plan.