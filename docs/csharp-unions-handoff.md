# Handoff — C# 15 union projection

Written 2026-08-25. Context for whoever picks this up next.

Design lives in `docs/csharp-unions.md`. Bugs live in `Issues.md`. This file covers what is
done, what is next, and the things that cost time to discover.

---

## 1. State

**Baseline.** `cargo nextest run -p interoptopus_csharp` — **77 passed, 2 skipped, 0 failed**,
5 binaries, measured 2026-08-26 **in a transaction worktree, not on `master`** — re-measure before
treating it as master's number. `master` was `d477f843` when this was written and has since moved.
Whole workspace was **174 passed, 3 skipped** at `318256c7`, not re-measured since; it is now
higher by the 13 tests this session added to `interoptopus_csharp`.

**Record the command, the scope and the commit with the number.** Six counts exist in this repo's
history and most of them said none of the three. **Resolved 2026-08-26:**

| Count | Command, scope, commit |
|---|---|
| 174 passed, 3 skipped, 38 binaries | `cargo nextest run`, whole workspace, at `318256c7` |
| 64 passed, 2 skipped, 5 binaries | `cargo nextest run -p interoptopus_csharp`, at `50e0788e` |
| 77 passed, 2 skipped, 5 binaries | same command and scope, in a transaction worktree 2026-08-26; base commit not recorded |
| 33 (30 passed, 1 failed, 2 ignored) | `cargo test -p interoptopus_csharp`, per `ccb105a2` |
| 12 unit + 31 integration + doctests | an earlier figure here; command, scope and commit all unrecorded |
| 3 of **62** failed, fresh worktree | `prepare_plugin`'s doc comment, `tests/mod.rs:108`; command unrecorded, and 62 matches no other count here |

64 → 77 is **13**: 4 case-type tests, 3 constructor tests, 6 union-member tests. `50e0788e` is
docs-only after `318256c7`, so the 64 is also `318256c7`'s package count — the package and
workspace figures were never in conflict, they answered different questions. `cargo test` also
counts nine doctests nextest does not run. **A bare pass count is not a baseline** — the command,
the scope and the commit are what make it one. The 62 is still unscoped; fixing it means editing
`tests/mod.rs`, which a docs change cannot reach.

### Plan state

**Status lives in `docs/csharp-unions.md` § Todo/Remaining. That table is the single source. This
section names the front and deliberately does not restate it** — the two drifting apart is what
cost time this morning.

Open front: **3d** (`[Union]` + `IUnion`, gate 3c ✔) and **3e** (implicit conversion, gate 3b ✔).
**4** (`ToUnmanaged`/`AsUnmanaged` empty guard) has no gate and can go in parallel. Also open and
**not covered anywhere below**: 4c, 4d's serializer half, 5c–5i, and 6. 4c is a soundness
obligation rather than a preference, so check it against what 3c already emits before sequencing
it behind the attribute work.

Landing log for this session — a record of what shipped, not a second status table:

| Item | Commit | What landed |
|---|---|---|
| 1d | `f5057d4b` | Six wire sites emit `v.stem`; `cs_type_name` sanitises |
| 3b, 3f | `4a19b0e3`, tests `9d6905bd` | Nested case types, `public`, union-projected enums only |
| 3a | `f630e225` | Private parameterless ctor on class-backed enums; changelog entry |
| 3c | `bdd13b53` | `HasValue`/`Value`/`TryGetValue`, `_hasValue` writes, `can_carry_payload` |
| — | `d477f843` | `WarningsAsErrors=CS0169;CS0414` gate in `Directory.Build.props` |

**`Issues.md` as of `d477f843`.** Open — `2a6da76a`, `4e9a17c3`, `7c8cb22e`, `79be256e`,
`b4e07f12`, `5d1ae4c7`. Closed — `09b82d44`, `ccb105a2`, `1383b84b`, `31248473`, `c33b9cf5`,
`e235bc7d`, `8f4c1e2a`.

### The three things worth knowing before you start

**1. `5d1ae4c7` is now an *eleven*-site gate, and I made it worse.** Every enum output pass opens
with the same inline `match type_kind { DataEnum(e) => e, Result(_, _, e) => e, Option(_, e) => e,
_ => continue }`. `union_names::data_enum()` already implements exactly this and nothing calls it.
I added two more passes this session (`body_case_types`, `body_union_members`), each with its own
copy. If you add a twelfth, consider adopting the helper first — it is a mechanical change and the
issue has the site list.

**2. Item 1 still gates 4b and is still unmeasured.** Positions (a) `InvalidOperationException`
and (c) `?? default` were never reproduced; only (b) was, and it showed the NRE fires *before* any
marshaller, at `body_as_unmanaged.rs:47` in the enclosing composite. `b4e07f12` records the
present-tense defect. Reproduce (a) and (c) before choosing, and do not close `b4e07f12` first.

**3. `ResultVoidVoid` regained union machinery in 3c, deliberately.** If you see it carrying four
empty case types and wonder whether that is bloat: it is the eligibility predicate asking *can a
variant carry a payload* rather than *does one*. See §8.

### Snapshot workflow

Accept **per step**, not deferred. The accept must run in the *transaction worktree*, not the real
checkout — `ccb105a2`'s "real checkout only" rule applied to LFS stubs, and LFS is gone since
`1383b84b`.

```powershell
cd <worktree>
$env:INSTA_UPDATE='always'; $env:TRYBUILD='overwrite'
cargo test --workspace --all-features --no-fail-fast
Remove-Item Env:INSTA_UPDATE, Env:TRYBUILD
Get-ChildItem -Recurse -Filter *.snap.new | Remove-Item
```

**Delete stray build logs before committing.** A `dotnet build > build.txt` left in the worktree
was picked up by a commit and blocked the checkout with *"Untracked working tree file … would be
overwritten by merge"*. The commit itself had already validated; only the checkout failed.

---

## 2. Scope, and what this is not

**Read `docs/csharp-unions.md` § "Two layers, two rules" before writing any code.** It is the
second section of that file and it is the thing most likely to be misread.

The short version: this work governs the **generated layer** only. Every `DataEnum` goes through
the union machinery, unit-only ones included, *not* because a union is better for a scalar choice
but because the generator cannot distinguish a scalar choice from a payload alternative — that is
domain knowledge it does not have, so any eligibility rule based on variant shape would be a
guess.

That is not a mandate for the consumer API. A consumer decides per type: payload or state
alternatives → union, scalar choice → `enum`, bit combinations → `[Flags]`, product data →
`record`. The test is whether the product type permits impossible states — a record with a
target, a referent and two independent `IsX` booleans admits combinations that cannot occur, and
should be a union; a closed set of numeric values with no per-case payload should stay a plain
`enum`, where bolting a `.SomethingCase` onto it would be a straight regression. Where a
unit-only enum's generated representation becomes union-like, translate at the boundary rather
than propagating case types outward.

Note the direction of that dependency: interoptopus decides what it emits from the C#
specification and the Rust inventory. A consumer may motivate a shape by showing it occurs in
practice, but never constrains the projection. See `csharp-unions.md` §0.

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

### ~~BLOCKER: the wire generator still emits raw variant.name~~ — landed in `f5057d4b`

**This is done.** The section is kept because its reasoning still governs new code.

Six sites in `wire/mod.rs` emitted `v.name` — the raw Rust identifier — instead of `v.stem`, the
collision-resolved name `union_names` allocates. `cs_type_name` also skipped `sanitize_rust_name`.
Both are fixed, and `pass::output::common::wire::cs_names` now owns the resolution, with unit tests
covering collision, tag-not-position lookup, and the wrapped-`DataEnum` path.

**The rule that outlives the fix:** every emitted member name comes from `v.stem`, never `v.name`.
`union_names` is the single naming authority. If you add a pass that names anything per-variant,
take the stem — `c33b9cf5` and `4e9a17c3` are both instances of something deriving names
independently, and both were defects.

`7c8cb22e` remains open: enum variant names are still never sanitised at the model layer.

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

## 5. Next: 3d and 3e, then 4

Step 3's *representation* work is done. What remains in Step 3 is the attribute layer (3d) and the
implicit conversion (3e); then Step 4's marshalling.

**3d — `[Union]` + `IUnion`.** The generated type already declares an interface list in some
cases (`IResult<Unit, Unit>`), so this joins rather than replaces. Verified by compilation on
preview 7: the `[Union]` declaration shape, constructors and `Value` compile under C# 13 *and*
preview; the implicit conversion (`Shape s = new Shape.CircleCase(1.0)`) is **preview only**,
failing with CS8652 "unions" on stable. Union pattern matching with no default arm is preview only
and produces no CS8509, so exhaustiveness works.

**Preview-only syntax is settled, not an open question.** `LangVersion=preview` is set in
`Directory.Build.props` and landed with R in `9d664613`, and a .NET 11 preview SDK is already
required to run the tests at all. The CS8652 note above is therefore a fact about *consumers*
compiling on stable, not a decision this repository still has to take — which is why it is not in
§6.

**3e — implicit conversion.** Gate 3b, satisfied since `4a19b0e3`, so this is workable now,
alongside 3d rather than after it. It shrank to a claim rather than an emission: the compiler
synthesises the conversion from the generated constructor, so 3e reduces to "constructors are
public and single-parameter" and nothing is emitted for it.

**4 — empty guard on `ToUnmanaged`/`AsUnmanaged`.** No gate. Can start now.

**4a — `ToManaged` via case constructors.** This replaces the whole method body. Note that 3c left
a deliberate stopgap there: `_managed._hasValue = true;` immediately after the `_variant` copy.
4a's validated switch establishes the flag implicitly, so delete the stopgap rather than keeping
both.

### What 3c actually emits, so you can read the output

For a **struct-backed** union:

```csharp
public bool HasValue => _hasValue;

public object? Value => !_hasValue ? null : _variant switch
{
    0 => new ACase(),
    1 => new BCase(_B),
    _ => null,
};

public bool TryGetValue(out BCase value)
{
    if (_hasValue && _variant == 1) { value = new BCase(_B); return true; }
    value = default;
    return false;
}
```

For a **class-backed** union, `HasValue => true`, `Value` has no `!_hasValue ? null :` prefix, and
`TryGetValue` has no `_hasValue &&` — there is no such field, because `default(E)` is a null
reference and every non-null instance is valid.

`Value` boxes a `readonly record struct` per access, so it is **value-stable, not
reference-stable**. That is deliberate: an eager `_boxed` field would allocate inside `ToManaged`,
on every boundary crossing. `TryGetValue` does not allocate.

### The eligibility rule, and the predicate that implements it

A `DataEnum` with no payload-**capable** variant receives no union machinery — no case types, no
`Value`/`HasValue`/`TryGetValue`, and since 3c, no `_hasValue` field either. The rule is per
*enum*, not per variant: within a union-projected enum, **every** variant gets a case type,
unit variants included, because an empty case type is what keeps a mixed enum exhaustive.

The predicate is `v.can_carry_payload`, a field on the C# `Variant`. **It is not `v.ty.is_some()`,
and the difference is not cosmetic** — see §8.

Unit-only enums keep their struct, `Unmanaged` mirror and marshaller. Projecting them as plain C#
`enum`s is option C in `79be256e`, still deferred: it needs an unverified blittability claim and
closed enums, which did not ship in C# 15.

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

**`dokono test selection fallback: affected_source_identity_invalid` is benign.** It appears in
`commit_transaction`'s validation warnings on essentially every change to a non-root source
module, and it **fails open**: `DokonoTestPlan::fallback` sets status `WorkspaceFallback`, clears
`subjects` and `selected_step_ids`, and keeps `original_test_step`, so nothing is narrowed and the
full step runs. `WorkspaceFallback` and `Skipped` are separate states and only `Skipped` emits its
own warning, so a fallback warning never means tests were dropped.

The trigger is narrow: the identity builder in `dokono_test_plan.rs` derives a `binary_id` only
for conventional shapes — `tests/`, `benches/`, `examples/`, `src/main.rs`, `src/lib.rs`. Anything
deeper under `src/` matches none of them, `SourceTestIdentity::new_named` rejects the identity, and
the error maps to this code. The real limitation of the fallback is **over**-selection rather than
under-selection; `source_name_fallback_can_overselect_same_named_tests_within_one_binary` asserts
precisely that. Nothing to fix; the only loss is the speed benefit of narrowing.

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

**PowerShell**: `2>&1` on a native command turns stderr into `ErrorRecord` objects, so `cargo`'s
ordinary progress output renders in red with `NativeCommandError` and a `+ CategoryInfo` block —
it looks exactly like a build failure and is not one. The first `Compiling` line takes the
decoration and the build continues normally. It also pollutes any file you `Tee-Object` into. Use
`cmd /c "cargo nextest run > out.txt 2>&1"` so the redirection happens outside the PowerShell
pipeline, or `*>&1` if you must stay in it.

**PowerShell**: use here-strings (`@'` … `'@`, delimiters alone on their line) for commit messages.
Escaped quotes inside a double-quoted string terminate it early and scatter the message across
`git add`. Bash heredocs (`<<'@'`) do not parse in PowerShell at all. Piping a here-string into
`git commit -F -` prepends a UTF-8 BOM to the subject line on PS 5.1; write the file with
`[IO.File]::WriteAllText(path, $msg, (New-Object Text.UTF8Encoding $false))` and pass that instead.

---


### MCP tooling notes, 2026-08-26

**FileMcp now has transactions, and they are the right tool for docs-only changes.**
`FileMcp:begin_transaction` takes a `chatUrl`, plus `folders` and `extensions` scope — so a docs
transaction cannot accidentally carry a code edit. Commit is async: `commit_transaction` queues,
then poll `get_commit_status`.

**The older note that `add_markdown_section` returns "No approval received" is stale.** It works.

Two quirks in `replace_markdown_section`: pass the heading as a **leaf**, not a full path, and put
the `##` heading inside `content` — the `title` parameter deletes the old heading without writing
a new one, orphaning the section under its predecessor. Both tools take a dry run first, which
returns the hash to pass back as `expectedTransformedHash`.

**Root aliases are per server, and the Rust editor cannot reach a FileMcp worktree at all** — the
call fails outright. `$N` in one server is not `$N` in the other. For a change mixing code and
docs, use one Rust-editor transaction rather than two, or they race on the same repo.

**Measuring C# warnings needs `-t:Rebuild`.** An incremental build recompiles nothing and emits no
warnings, and `CSharpEditor:build_diagnostics` returns *errors* — a clean result from either proves
nothing about warnings. The reliable form, per the `2>&1` note above:

```powershell
cmd /c "dotnet build -t:Rebuild -v:n > build.txt 2>&1"
Select-String -Path build.txt -Pattern 'warning [A-Z]+\d+' | ForEach-Object { $_.Matches.Value } | Group-Object | Sort-Object Count -Descending
```

**Delete `build.txt` before committing** — a stray one was picked up by a commit and blocked the
checkout with *"Untracked working tree file … would be overwritten by merge"*.

**`CSharpEditor:list_git_commits` without a `project` searches CSharpMpc**, not interoptopus,
because that server's active root is elsewhere. Pass a project path inside the repo you mean.

**`search_and_replace` with a bare `file_glob` matches every file of that name.** `body_unmanaged.rs`
and `mod.rs` each exist under both `enums/` and `composites/`; a batch aimed at one silently edited
the other. Anchor on text unique to the intended file, and read the dry run's file list before
applying.

**`dokono test selection fallback: affected_source_identity_invalid` is benign** — see the entry
above; it fires on essentially every change to a non-root source module and fails open.

**A first run in a fresh worktree can fail the concurrency test.** `prepare_plugin`'s doc comment
records it: *"a fresh worktree fails three of sixty-two on the first run and passes on the second."*
Changing `Directory.Build.props` invalidates every plugin build and reproduces exactly this. Re-run
before investigating.
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

### 2026-08-26 — the eligibility predicate asked the wrong question

**The worst error of the session, and it shipped through 3b before anyone caught it.**

I wrote the union eligibility check as `v.ty.is_some()` — *does this variant carry a payload*. The
correct question is *can* it: whether the **declaration** has a payload slot. Those coincide
everywhere except one shape, which is why every test passed.

`fallback.rs::resolve_payload` maps a `()` payload to `None`. So `Result<(), ()>` — which declares
`Ok(T)` and `Err(E)`, both payload-carrying positions — reads as entirely payloadless downstream and
became indistinguishable from `Color::Red`. `ResultVoidVoid` was therefore classified unit-only and
stripped of its union projection, while `Result<u32, Error>` kept it. **The same shape, decided by a
type argument.**

Three compounding mistakes:

1. **I inferred the meaning of `ty` instead of reading what populates it.** `enum_variants.rs` does
   not erase — a `Tuple(())` there resolves to a void `TypeId`. Only `fallback.rs` erases. I read
   the first and assumed the second. The handoff's closing line is *read before inferring*, and
   `8f4c1e2a` records me doing the same thing that morning with `nullable.rs`.
2. **The plan already named the right mechanism** — "`VariantKind::Tuple` versus `Unit`, visible in
   the inventory" — and I had quoted that line into the docs hours earlier, then implemented the
   other thing.
3. **When challenged I defended it badly**, invoking the consumer-layer "closed set of numeric
   values" rule while discussing `Result`. That rule governs a *different layer*; the section is
   titled "two rules" precisely because they are separate. I then proposed special-casing
   `Result`/`Option`, which would have papered over the erasure and still misclassified a
   hand-written `enum E { A(()), B }`.

The fix is a model-layer field, `Variant::can_carry_payload`, set from `VariantKind::Tuple` in
`enum_variants.rs` and from the payload-carrying position in `fallback.rs`. Five predicate sites
read it.

**If you touch eligibility, the guard test is
`enum_union_members::eligibility_asks_can_carry_not_does_carry`.** It is the *only* fixture that
separates the two predicates — reverting to `ty.is_some()` passes the entire rest of the suite.
Verified by mutation: exactly one test fails.

### 2026-08-26 — I filed an issue on a diagnosis I had not finished

`8f4c1e2a` went in at severity high asserting that no project compiles generated C#, that
`benches/dotnet` was committed output, and that the feedback loop was severed at both ends. All
three were false, and I disproved them myself within the hour:

- The twelve plugin fixtures compile generated C# on **every** `cargo test`. The loop is complete;
  `build_and_stage` just checks the exit status and runs `-v q`, so warnings are discarded.
- `benches/dotnet/Interop.cs` is gitignored and **regenerated on every run** by
  `reference_project::interop`, which writes both it and `Bindings/`.
- `benches/dotnet` is *drifted*, not structurally broken — `Benchmark.cs` frozen since 2026-04-14.

The measurement (19 CS0169, all `_hasValue`) was sound throughout. The diagnosis around it was not.
The issue is now corrected in place with the three errors stated explicitly rather than quietly
edited, so the record shows what was measured versus what was inferred from it.

**Two process notes from that.** I twice reported build results from an *incremental* build that
recompiled nothing — `build_diagnostics` returns errors, not warnings, and a no-op build emits
neither. Use `-t:Rebuild` when measuring warnings. And an unscoped `CSharpEditor:list_git_commits`
returns **CSharpMpc** history, not interoptopus, because that server's active root is elsewhere —
pass a `project` inside the repo you mean.

