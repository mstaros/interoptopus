# Handoff — C# 15 union projection

Written 2026-08-25. Context for whoever picks this up next.

Design lives in `docs/csharp-unions.md`. Bugs live in `Issues.md`. This file covers what is
done, what is next, and the things that cost time to discover.

---

## 1. State

Committed on `master`, suite green (12 unit + 31 integration + doctests, 2 ignored):

| Commit | What |
|---|---|
| `09b82d44` fix | Proc macro resumes implicit discriminants from the previous value |
| `c928d53e` | Discriminant moved onto `Variant.tag`; three index-as-tag sites fixed |
| `9d664613` | net11 retarget, `LangVersion=preview`, plugin DLLs rebuilt |
| `bbff2055` | Plan updated for net11-everywhere |
| *(union_names commit)* | `union_names` model pass + allocator, 12 unit tests |
| *(wiring commit)* | Pass wired into both pipelines |
| `2bdbf054` | All nine name-deriving sites emit from `v.stem` |

`Issues.md`: `09b82d44` closed. Open — `2a6da76a`, `ccb105a2`, `1383b84b`, `7c8cb22e`.

Plan todo: items 0–0e and 1–1c done. **Item 3 is next.**

---

## 2. Scope, and what this is not

**Read `docs/csharp-unions.md` § "Two layers, two rules" before writing any code.** It is the
second section of that file and it is the thing most likely to be misread.

The short version: this work governs the **generated layer** only. Every `DataEnum` goes through
the union machinery, unit-only ones included, *not* because a union is better for a scalar choice
but because the generator cannot distinguish a scalar choice from a payload alternative — that is
domain knowledge it does not have, so any eligibility rule based on variant shape would be a
guess.

That is not a mandate for the consumer API. GixSharp decides per type: payload or state
alternatives → union, scalar choice → `enum`, bit combinations → `[Flags]`, product data →
`record`. The test is whether the product type permits impossible states. `GixHead` should be a
union because its record admits combinations that cannot exist; `GixObjectType` should stay a
plain `enum`, and exposing `GixObjectType.CommitCase` would be a straight regression. Where a
unit-only enum's generated representation becomes union-like, translate at the boundary rather
than propagating case types outward.

Do not add an eligibility gate to interoptopus to try to enforce the consumer-layer rule. It
cannot know enough to apply it.

---

## 3. Conventions this work established

**`v.stem`, never `v.name` — but this is not yet true everywhere.** See the blocker below
before relying on it.

`lang::types::kind::Variant` carries `stem` and `case_type` alongside `name`. Every emitted
member should derive from `stem`:

| Member | Form |
|---|---|
| factory | `{stem}` |
| check | `Is{stem}` |
| accessor | `As{stem}` |
| payload field | `_{stem}` |
| unmanaged helper | `Unmanaged{stem}` |
| case type (new) | `{case_type}`, normally `{stem}Case` |

### BLOCKER: the wire generator still emits raw `variant.name`

An earlier revision of this file claimed `name` was "only for diagnostics". **That is false.**
`crates/backend_csharp/src/pass/output/common/wire/mod.rs` emits it at six sites:

- `emit_enum_serialize` — `{val}.Is{}` and `{val}.As{}()`
- `emit_enum_deserialize` — `{enum_name}.{}(payload)` and `{enum_name}.{}`
- `emit_enum_size` — `{val}.Is{}` and `{val}.As{}()`

The nine sites under `pass/output/common/types/enums/*` were migrated; these were not.

**This is not a one-word fix.** `wire/mod.rs` takes `e: &interoptopus::lang::types::Enum` — the
*Rust inventory* type, which has no `stem` field at all. The root cause is one level up:
`WireCodeGen` holds only `rs_types: &RsTypes`, so no emitter in that file can reach the C# model.
Resolving it needs a model reference threaded into the struct. That is a design decision, not a
rename.

**There is a second family, from the same cause.** `cs_type_name` returns `ty.name.clone()` for
`Struct` and `Enum`, skipping the `sanitize_rust_name` that every path through `names.rs` applies
(line 134 is the catch-all: `_ => sanitize_rust_name(&ty.name)`). The comment at `names.rs:113`
does **not** license this — it says to resolve the Wire inner *name* from the Rust inventory, and
then still applies `sanitize_rust_name` and `rust_to_pascal` to it. Reading the source string from
`rs_types` is not the same as using the Rust name verbatim.

**Why no test catches it.** `stem == name` for every reference-project enum, because none
collide. The invariant is unenforced. A colliding enum would generate a managed type using
`IsFooVariant` while the wire serializer emits `IsFoo` — two halves of the same file
disagreeing, silently, and only for enums nobody has written yet.

**Fix this before item 3b.** Items 3b and 3c both *add* collision surface — `{stem}Case` is a new
collision class, and 3c reserves `Value`/`HasValue`/`TryGetValue` — so the rate of `stem != name`
goes up as Step 3 lands. Adding case types on top of a half-migrated naming layer buries the
inconsistency deeper. Tracked as `Issues.md` `4e9a17c3`; plan item 1d, which now gates 3b.

**When fixing, match variants by `tag`, not by index or name.** Since Step 0 both the Rust and C#
variant carry the same authoritative discriminant. Index is positional and brittle; name is the
thing being corrected.

**A third gap, same file, different contract.** `emit_enum_serialize` falls through to
`throw new InvalidOperationException("Unknown variant")`. Once item 3c consumes `_hasValue`,
every `IsX` returns false for a default struct union, so wire reaches that fallback for an empty
enum. Step 4 routes empty-state and corrupt-tag through `ExceptionForVariant()` with different
exception types; wire honours neither. Plan item 4d.

### Once that is done

**Templates must not re-sanitize.** `union_names` guarantees uniqueness over the exact strings
it produces. Any casing or escaping applied downstream breaks that guarantee. `wire` currently
breaks the same invariant from the other direction, by under-sanitizing off a different source.

**Names live on the variant, not in a side table.** Several output passes filter variants before
emitting — `body` keeps only disposable ones, `body_as_unmanaged` only payload-carrying ones — so
a parallel `Vec<VariantNames>` indexed positionally misaligns silently after any filter.

---

## 4. Three things that cost time

**A `DataEnum` reaches the model by three routes.** Directly as `TypeKind::DataEnum` from
`enum_variants`, and wrapped inside `TypePattern::Option(TypeId, DataEnum)` or
`TypePattern::Result(TypeId, TypeId, DataEnum)` for the shapes `fallback.rs` synthesises and
`type_map_patterns` installs. Match only the first and `Ok`/`Err` stay unresolved — the symptom
was `_` for every payload field and factory. `union_names::data_enum` / `data_enum_mut` handle
all three; use them rather than matching inline.

**`type_all` is a rebuilt copy of `type_kinds`.** Model data that output passes read comes from
`type_all`, but `type_all.process` derives its `Type` values from `type_kinds`. Writing to
`type_all` populates something that gets overwritten. **Write to `type_kinds`, and run before
`type_all.process`.** `union_names` sits immediately after `type_names.process` in both
pipelines for exactly this reason — moving it later silently breaks it.

**The convergence loop re-runs passes.** Guard on your own output being present, the way
`union_names` treats an empty `stem` as "unresolved". A pass that always reports `Changed` will
not converge.

---

## 5. Next: item 3

The design is written up in `docs/csharp-unions.md` Step 3. Summary of the load-bearing parts,
because several were argued over and reversed:

**Struct and class unions have different contracts. Do not unify them.**

- *Struct*: `bool _hasValue` in the managed partial only, never in `Unmanaged`. `default(T)` is a
  real value, so `Value` → null, `HasValue` → false, `IsX` → false, `AsX()`/`ToUnmanaged()` throw,
  `Dispose()` no-op.
- *Class*: no `_hasValue`. Add a **private parameterless constructor** so `new EnumX()` cannot
  produce a bogus variant-zero instance; `default` is a null reference and every non-null instance
  is valid, so `HasValue` is constant `true`. This is a breaking change for anyone calling
  `new EnumX()`.

`struct_class.is_struct()` decides which.

**`Value` materialises on access.** `_variant switch { 1 => new BCase(_B), ... }`. Do *not* add an
eagerly-populated `_boxed` field — it allocates in `ToManaged`, i.e. on every enum crossing the
boundary, which is the cost this whole design exists to avoid. Consequence: `x.Value != x.Value`
by reference. That is acceptable; the spec constrains the *type* of `Value`, not its identity, and
`TryGetValue` is the path patterns actually take.

**`ToManaged` should construct, not mutate.** Emit a validated switch returning through the case
constructors rather than `new E()` + field assignment. Uniform across struct and class, sets
`_hasValue` implicitly, makes an invalid native tag unrepresentable. For struct constructors mind
definite assignment — `this = default` first.

**Every tag consumer must respect `_hasValue`**, not just `Dispose()`: `IsX`, `AsX`, `Value`,
`ToString`, `ExceptionForVariant`, `ToUnmanaged`, `AsUnmanaged`.

**Interface list**: build a `Vec<String>` and join it. `body.cs` already nests conditionals for
`IResult` and `IDisposable`; adding `IUnion` to that inline is how comma bugs happen.

---

## 6. Open decisions

**Exception split.** `InvalidOperationException` for a default struct union versus
`InteropException` for a corrupt native tag. Reviewed both ways; current position is to split,
because a default struct union is a legal C# state while `InteropException` in this codebase means
"severe error, should never happen". Route it through `ExceptionForVariant()` so there is one
helper. Not implemented.

**`ToString()` on empty** returns `<empty>`. Decided, not implemented.

**Null reaching the marshaller for a class union** — today's `NullReferenceException`, or a
deliberate `ArgumentNullException`/`InteropException`? Undecided.

---

## 7. Environment and workflow

**Transactions work again** — `Issues.md` `1383b84b` is closed. They used to fail for *any*
change, including an empty one: the MCP commit pipeline built its candidate tree without running
the Git LFS clean filter, so it would have stored content where a pointer belonged and the guard
correctly refused. Nothing in this repo is LFS-tracked now, so there is nothing to trip on.
Verified by `2b1825f7`, a transaction that built a real candidate tree and integrated. The
tooling defect itself is unfixed, so it returns if LFS tracking ever does.

Two things worth knowing when a commit looks like it failed. `commit_transaction` can return a
transport error to the client *after* succeeding server-side — check `get_commit_status` before
retrying, or you will retry against an already-integrated transaction. And a no-op transaction
proves nothing about the pipeline, because it never constructs a candidate tree; only a change
that writes one does.

**LFS.** Removed. Nine extensions were tracked, three of them text (`*.json`, `*.snap`, `*.svg`)
and none large — `global.json` is 63 bytes, the largest committed DLL was 30 KB. `.gitattributes`
now classifies by text versus binary. If you reintroduce tracking, expect `1383b84b` back.

**Snapshots.** `just` is not installed. The accept incantation is:

```powershell
$env:INSTA_UPDATE='always'; $env:TRYBUILD='overwrite'
cargo test --workspace --all-features --no-fail-fast
Remove-Item Env:INSTA_UPDATE, Env:TRYBUILD
Get-ChildItem -Recurse -Filter *.snap.new | Remove-Item
```

`--no-fail-fast` matters: without it a single failure stops later crates and their snapshots go
unaccepted while appearing to have been handled.

**Plugin DLLs are no longer committed and no longer built by hand.** `define_plugin!` builds each
plugin's `.csproj` during `cargo test` and stages the DLL into `_plugins/`, so it cannot drift
from the interop sources and `ApiMismatch` no longer has a way to happen. `build-dotnet-plugins`
and its `_bdp_ref`/`_bdp_p` helpers were deleted with the recipes. A .NET 11 preview SDK is
therefore required to run the tests at all.

`define_plugin!`, `load_plugin!` and `dll_path_for` all route through `ensure_plugin_built`,
which builds at most once per process. That guard is load-bearing: cargo runs a plugin's define
and load tests on parallel threads, and Windows refuses to overwrite a DLL the .NET runtime has
mapped (`os error 32`). `dll_path_for` is the easy one to miss — several tests reach the DLL
through it rather than through `load_plugin!`.

**That guard was per *process*, which the commit validator's runner defeats.** `cargo-nextest` gives each test its own process, so the `Mutex<HashSet>` coordinated nothing and several processes compiled the same `.csproj` into the same `obj/`; the loser died with `CS2012`, the output file held by the winner. Reproduced by bisection on an unchanged tree — single-threaded passed, a five-test filter passed, the full run failed on four different plugins. Now guarded by a lock file at `_plugins/.lock-<name>` held across build-and-stage, with age-based reclamation so a killed test cannot wedge later runs. Two things to know: the reclamation path has no test, and `cargo test` and `nextest` do not run the same set — `cargo test` counts nine doctests that nextest does not run at all, so "the suite passes" means different things depending which you ask.

**`prepare_plugin` now owns generation as well as build and stage.** It used to be split —
`define_plugin!` generated the interop sources, `ensure_plugin_built` compiled them — so a
loader reaching a plugin first compiled against whatever happened to be on disk. On a fresh
checkout the `Interop*.cs` files are gitignored and therefore absent, so the build failed with
`CS0246` rather than producing a stale DLL. Measured: a fresh worktree failed three of
sixty-two on the first run and passed on the second. `dll_path_for` is now
`dll_path_for::<P>`, and every path to a staged DLL generates first, so arrival order no longer
matters. Two things to keep in mind if you touch it: the snapshot assertion stays in
`define_plugin!` (moving it into the shared helper would make every loader assert a snapshot it
did not ask for), and writes go through `Multibuf::write_buffers_to_if_changed`, not
`write_buffers_to`. The latter rewrites unconditionally, which bumps mtime, which forces a
rebuild, which makes the built DLL newer than the staged one and re-fires the copy that
`stage_is_current` exists to avoid. The content check also has to live in `Multibuf` rather
than in the harness, because the per-buffer `Overwrite` policy is private and a loop over
`iter()` would silently clobber `Overwrite::Never` files.

**Plugin fixtures must not take third-party `PackageReference`s.** `_plugins/` staging copies the plugin DLL and not its dependencies, so a fixture with one passes only on a checkout where an earlier run left the dependency behind — a false green on any clean clone. The `wire` fixture had `Newtonsoft.Json` and passed for exactly that reason; it is now on `System.Text.Json`, which is in the BCL. `Newtonsoft.Json` is gone from the repository.

**Toolchain**: .NET 11 Preview 5+ required. Verified on `11.0.100-preview.7.26381.103`.
`rt/dynamic.rs` pins the hostfxr runtime config at `11.0.0-preview.1` — deliberately a
pre-release, because hostfxr will not roll forward from a release request to a pre-release
runtime. Leave it. CI installs the SDK on **every** OS; it used to be gated to Linux and pinned
to 10.x, which the runtime pin above made unusable.

**MCP tool quirks worth knowing.** `Rust editor:str_replace` is parser-aware and will not match a
pattern spanning categories — a pattern containing a string literal or a `//` comment silently
returns zero matches. Use `search_and_replace` (regex) for those, but note its replacement does not
honour `\n`, so multi-line inserts need `str_replace` with a pure-code pattern, or `write_file`.
`FileMcp:write_file` HTML-escapes XML content — `<Project>` became `&lt;Project&gt;` and had to be
repaired. `add_markdown_section` returns "No approval received" with no prompt shown;
`replace_markdown_section` works on the same file, but drops a trailing `---` separator unless you
include it in the replacement, and its `expectedTransformedHash` is bound to the exact content you
dry-ran — edit the text and you must dry-run again. Root aliases are **per server**: FileMcp's
`$N` and the Rust editor's `$N` are different registries, so a transaction worktree alias from one
cannot be resolved by the other.

**PowerShell**: use here-strings (`@'` … `'@`, delimiters alone on their line) for commit messages.
Escaped quotes inside a double-quoted string terminate it early and scatter the message across
`git add`. Bash heredocs (`<<'@'`) do not parse in PowerShell at all. Piping a here-string into
`git commit -F -` prepends a UTF-8 BOM to the subject line on PS 5.1; write the file with
`[IO.File]::WriteAllText(path, $msg, (New-Object Text.UTF8Encoding $false))` and pass that instead.

---


## 8. Where I was wrong

Recorded because the same traps are still live.

I asserted three times from `Issues.md` prose without measuring — a "29 of 33 failing" blocker that
had been retracted, a claim that worktrees are LFS-blind, and then an over-correction saying the LFS
hazard was imaginary when the commit guard proves it is not. Measure; it was cheap every time.

I predicted the `v.stem` migration would be byte-identical, and it produced nine failures. Then I
"fixed" it by changing the write target without changing the ordering, which made it worse. Both
were assumptions about where the model is read from that reading `type_all.process` would have
settled in one call.

I said `Unmanaged{stem}`, `Is{stem}` and `As{stem}` were names union projection introduces. They are
already emitted today, which means `Foo`/`IsFoo`, `Foo`/`AsFoo` and `Foo`/`UnmanagedFoo` are
*pre-existing* broken output, not new risk. `union_names` fixes them as a side effect.

The step-2 inventory listed `.csproj` files and the Justfile and called that the framework retarget.
It missed `rt/dynamic.rs`, which pins the runtime the host boots — the one that actually mattered.
When changing a framework here, ask what pins it at *runtime*, not just at build time.
