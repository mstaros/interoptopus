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
status: open
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
again after `pull_target` onto `4fac2166`. Identical detail both times.

### Diagnosis

The LFS **smudge** filter runs when the transaction worktree is created — content is
materialized correctly. `r#mod__reference_project__interop.snap` is 848,675 bytes in the
worktree, identical to the main checkout, and all 12 DLL-loading plugin tests pass there.

The LFS **clean** filter does not run when the candidate tree is built. Every LFS-tracked path
is therefore staged as raw bytes rather than as a pointer, and the commit guard correctly
refuses. This suggests the candidate tree is assembled by writing objects directly rather than
through `git add`, which would bypass `filter.lfs.clean`.

### Impact

**Blocks every transaction in this repository, regardless of what it changes.**

`.gitattributes` tracks `*.snap` and `*.dll`. The `deferred changes classified` event lists 68
such paths; none belong to the change under test. `exceptions.dll` is simply the first path the
guard trips on. A transaction with an empty change set would fail identically.

The guard is doing the right thing — silently un-LFS-ing 68 binary and snapshot paths would be
far worse than a failed commit. The bug is upstream of it.

### Workaround

Copy the changed files from the transaction worktree into the real checkout and commit with
plain `git`, where the clean filter runs normally. Verified: the four files from `086e4102`
copied across, `git status` showed exactly those four, and the full suite stayed green
(31 passed, 0 failed, 2 ignored, plus doctests).

### Note

Not an interoptopus bug — it is a defect in the MCP transaction tooling, recorded here because
it blocks the execution model assumed by `docs/csharp-unions.md`, which planned to do the union
work in transactions throughout. Until this is fixed, that plan's steps must land through the
real checkout.

Two smaller observations from the same investigation:

- The failure detail is only reachable through the server log. `get_operation_log` omits
  `truncated_result`, so the operation log alone gives no actionable reason for the failure.
- `add_markdown_section` on the FileMcp server returns `No approval received` with no approval
  prompt shown to the user, while `replace_markdown_section` on the same file succeeds. Possibly
  related tooling inconsistency; recorded here only so it is not lost.