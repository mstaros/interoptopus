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
## Enum variant names are never sanitized, so a C# keyword variant emits uncompilable bindings

```issue
id: 7c8cb22e
kind: bug
severity: low
status: open
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