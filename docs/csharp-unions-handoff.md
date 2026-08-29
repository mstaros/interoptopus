# Handoff — C# 15 union projection

Written 2026-08-25. Context for whoever picks this up next.

Design lives in `docs/csharp-unions.md`. Bugs live in `Issues.md`. This file covers what is
done, what is next, and the things that cost time to discover.

---

## 0. Start here — what this is, and why

You are almost certainly starting with no context. Read this section before anything else.
`csharp-unions.md` opens mid-argument, and **`CLAUDE.md`'s pass-directory tree is stale
throughout** — it predates this work. Both `model/` and `output/` are shown flat when each is
split into `rust/`, `dotnet/` and `common/`; several listed files do not exist; and several that
do are missing. Treat the whole tree as unreliable and list the directory instead. The map below
was built by listing, 2026-08-27.

### Before you touch anything

**Toolchain: a .NET 11 preview SDK is required or nothing runs at all.** Verified on
`11.0.100-preview.7.26381.103`. `rt/dynamic.rs` pins hostfxr at `11.0.0-preview.1`, deliberately a
pre-release; leave it. `LangVersion=preview` is set in `Directory.Build.props`.

**There are two tracks, and the plan's front is not the objective. Read this before picking one.**

| | Track A — the plan's front | Track B — the objective |
|---|---|---|
| What | Item 4, then 4a: reject the empty struct state in `ToUnmanaged`/`AsUnmanaged`, then rebuild `ToManaged` on the case constructors | The projection pass, steps two and three: give the model a notion of *how a type is projected*, then route unit-only enums to a plain C# `enum` |
| Why it is listed first | It is the next unstarted row of `csharp-unions.md` § Todo/Remaining | It is what the repository owner asked for and it has **not happened** |
| Where specified | `csharp-unions.md` § Step 4 \(**not** §5 of this document, which only names it\) | §9 of this document |
| Gate | none | step one landed in `1429746b` |

**If you do only one thing, do Track B.** Every `#[ffi]` enum is still emitted as a ~120-line
struct; `Color { Red, Green, Blue }` produces a discriminant field, an `Unmanaged` mirror and a
marshaller, exactly as before any of this work began. Union projection for payload-carrying enums
is finished and is the part that shows up in the landing log. **Plain enums are the part that was
asked for and is missing**, and a reader who takes "the front item is 4" at face value will plan a
week of marshalling work without noticing. That sentence used to be all this section said; it is
recorded here because it actually misled a fresh reader.

Track A is not wrong and is not blocked — it is the correct next row if you are working the plan
in order. It just is not the thing anyone is waiting for.

**Status is `csharp-unions.md` § Todo/Remaining, and only there.** Where this document and that
table disagree, the table wins. Note that the table orders by dependency, not by priority, which
is the whole reason for the two-track note above.

**File map.** Everything below is under `crates/backend_csharp/`:

| What | Where |
|---|---|
| C# `Variant` / `DataEnum` types | `src/lang/types/kind/enums.rs` |
| Kind assignment | `src/pass/model/common/types/kind/{enum.rs, enum_variants.rs, patterns.rs}` |
| Synthesised `Result`/`Option` carriers | `src/pass/model/common/types/fallback.rs` |
| Name allocation, the single naming authority | `src/pass/model/common/types/union_names.rs` |
| Derived facts about a type, **including `projection`** | `src/pass/model/common/types/info/{managed_conversion, struct_class, disposable, nullable, projection}.rs` |
| Enum output passes | `src/pass/output/common/types/enums/*.rs` |
| Templates the passes render | `templates/common/types/enums/*.cs`, `templates/rust/header.cs` |
| Wire name resolution | `src/pass/output/common/wire/cs_names.rs` |
| Pass registration, **both** pipelines | `src/pipeline/rust/library.rs`, `src/pipeline/dotnet/library.rs` |
| **Test harness** — `prepare_plugin`, `define_plugin!`, `load_plugin!`, `dll_path_for` | `tests/mod.rs` (the file cited as `:108` and `:224` elsewhere in this document) |
| Test suites | `tests/{output, model, common, reference_project, reference_plugins, backend_plugins}/` |
| Other test entry points | `tests/{template.rs, extensions.rs, basic.rs}` |

### What this repository does

Interoptopus generates bindings so other languages can call a Rust library; this crate is the C#
backend. It works in **two directions**:

- **Forward interop.** A Rust library annotated with `#[ffi]` gets C# bindings generated. C# calls
  Rust.
- **Reverse interop.** `plugin!` declares a .NET *interface* in Rust; the backend emits the C#
  side, and Rust loads the managed assembly at runtime through `hostfxr`. Rust calls C#.

**The reverse-interop fixtures are the only thing that compiles the generated C#.** Everything
else compares generated text against `insta` snapshots, which pass happily on C# that does not
build. When 3d first landed, the reference snapshot **passed** — on already-accepted output —
while fourteen plugin tests failed with a hard compiler error. Never conclude "it works" from
snapshots.

### What this work is, and why

Project a Rust `enum` that carries data as a **C# 15 union** rather than the struct-with-a-
discriminant it used to be. `Result` and `Option` carriers go through the same machinery — they
are `DataEnum`s wrapped in a `TypePattern`.

Before, a Rust enum arrived as a struct with `IsCircle` / `AsCircle()` and a runtime throw.
Nothing checked that a consumer handled every variant:

```csharp
if (s.IsCircle) return "circle";
if (s.IsRect)   return "rect";
throw new InvalidOperationException();   // silently reached when a variant is added
```

Add a variant, regenerate, and that code still compiles — it just starts throwing. With union
projection the compiler checks it instead, and a `switch` needs no default arm; add a variant and
every incomplete switch warns at compile time, at the call site. That is the point of the
exercise.

**Why hand-rolled `[Union]` and not the `union` keyword.** `public union Shape(Circle, Rect);`
lowers to a struct with a single boxed `object?` field — no discriminant, no explicit layout, an
allocation per value. Unusable here, because the generated type must match Rust's layout byte for
byte: `[StructLayout(LayoutKind.Explicit)]`, `_variant` at `FieldOffset(0)`, a memcpy crossing.
`[Union]` on a hand-written type is the same feature's other path; the specification says user
code may store contents any way it likes, and its `IntOrBool` example is our exact shape.

**Why not closed hierarchies.** They give the same exhaustiveness, but `closed` implies
`abstract`, so every value becomes a heap reference — no layout, no memcpy, `default` is `null`.
A reasonable fit for a **managed-only** enum that never crosses the boundary (item 5h), not for
one that does.

### What is now done, and what that leaves

**Unit-only enums are emitted as plain C# `enum`s.** As of `ca6aafa`, `Color { Red, Green, Blue }`
produces `public enum Color : byte { Red = 0, Green = 1, Blue = 2, }` instead of ~120 lines of
struct, discriminant field, `Unmanaged` mirror and marshaller. That is option C in `Issues.md`
`79be256e`, and it was the objective. **This section previously said the opposite** — it is kept
under a new heading rather than deleted so the change is visible to anyone who read the old one.

The rule has two halves and both are required: **no variant can carry a payload**, *and* the
discriminant is a type C# accepts as an `enum` base. `Primitive` has fifteen members and only
eight qualify — `nint`, `nuint`, `float`, `bool` and `void` do not — so a unit-only enum with a
non-integral discriminant keeps the struct. That is `Projection::Discriminant`, and it is the one
exclusion that stops the three-value enum collapsing to a boolean.

**What made it work was not the emitter.** `managed_conversion` classified every `DataEnum` as at
least `To`, never `AsIs`; making the plain-enum population `AsIs` is the lever, and composites,
slices and the mirror all followed without those emitters being touched. See §9.

**Still open on this front:** the pinning question in §9, and `Discriminant` has no member in the
reference corpus, so that branch is currently untested by anything.

---

## 1. State

**Baseline — and the count is not as firm as this document previously claimed.**
`cargo nextest run -p interoptopus_csharp` reported **77 passed, 2 skipped** in local runs on
2026-08-26 and 2026-08-27, but Guarded's validation runs of the *same worktree* reported **78
tests run**. Three observations, two different totals, no explanation found. Treat 77 as "about
seventy-seven, depending on how it is invoked" rather than as a number to assert. `master` is
`da741fa9`. Whole workspace was **174 passed, 3 skipped** at `318256c7`, not re-measured since.

**Record the command, the scope and the commit with the number.** Seven counts exist in this
repo's history and most of them said none of the three:

| Count | Command, scope, commit |
|---|---|
| 174 passed, 3 skipped, 38 binaries | `cargo nextest run`, whole workspace, at `318256c7` |
| 64 passed, 2 skipped, 5 binaries | `cargo nextest run -p interoptopus_csharp`, at `50e0788e` |
| 77 passed, 2 skipped, 5 binaries | same command and scope, local runs 2026-08-26/27 |
| **78 tests run**, 2 skipped | **same command, same worktree, under Guarded validation** — unexplained |
| 33 (30 passed, 1 failed, 2 ignored) | `cargo test -p interoptopus_csharp`, per `ccb105a2` |
| 12 unit + 31 integration + doctests | an earlier figure here; command, scope and commit all unrecorded |
| 3 of **62** failed, fresh worktree | `prepare_plugin`'s doc comment, `tests/mod.rs:108`; command unrecorded, and 62 matches no other count here |

64 → 77 is **13**: 4 case-type tests, 3 constructor tests, 6 union-member tests. `50e0788e` is
docs-only after `318256c7`, so the 64 is also `318256c7`'s package count — the package and
workspace figures were never in conflict, they answered different questions. `cargo test` also
counts nine doctests nextest does not run. **A bare pass count is not a baseline** — the command,
the scope and the commit are what make it one, and as the 77/78 pair shows, even those three are
not always sufficient. The 62 is still unscoped; fixing it means editing `tests/mod.rs`, which a
docs change cannot reach.

**On the 77/78 discrepancy: deliberately not filed, and here is the reasoning so you can
overturn it.** No wrong behaviour was ever observed — both runs pass, and the extra test is a
count difference in nextest's own reporting, not a test that fails in one and not the other. It
was seen three times on 2026-08-26/27, always as local-77 versus validation-78. **Do not use
either number as a regression signal**; compare pass/fail, not totals. If you ever see a *failure*
that appears under one runner and not the other, that is a different thing and worth filing — this
is not that.

**One measurement against that, from the canary commit `4aad161`.** The validation runner
reported *78 tests across 9 binaries, 3 skipped*. A local `cargo nextest run -p
interoptopus_csharp` is *5 binaries, 2 skipped*. Those are not the same selection, so the
premise recorded above — "same command, same worktree" — does not survive contact: validation
runs a broader set than the package-scoped command it was being compared against. This does not
fully explain 77 versus 78, and it is not offered as a resolution; it does mean the two figures
probably never counted the same tests, which is a better lead than "unexplained".

### Plan state

**Status lives in `docs/csharp-unions.md` § Todo/Remaining. That table is the single source, and
this section does not restate it.** An earlier version of this paragraph said exactly that and
then listed the open items anyway; the list drifted within a day, omitting 4b and 5. Go to the
table.

**Step 3 is complete except for one unverified claim.** Rust enums now project as C# 15 unions:
`[Union]`, `IUnion`, nested case types, public single-parameter case constructors, `Value`,
`HasValue`, `TryGetValue` — with the explicit layout and the memcpy crossing intact. **3e is
satisfied but unverified**: nothing is left to emit, but no fixture compiles an implicit
conversion, so the synthesis claim is untested.

**Two tracks — see §0's table before choosing.** Track A is item 4, the plan's next unstarted row.
**Track B is the objective**: the projection pass, whose step one landed in `1429746b`, ending in
plain C# `enum`s for unit-only enums — option C, still **not done**, and the thing that was
actually asked for. §9 specifies Track B; `csharp-unions.md` § Step 4 specifies Track A.

Landing log — a record of what shipped, not a second status table:

| Item | Commit | What landed |
|---|---|---|
| 1d | `f5057d4b` | Six wire sites emit `v.stem`; `cs_type_name` sanitises |
| 3b, 3f | `4a19b0e3`, tests `9d6905bd` | Nested case types, `public`, union-projected enums only |
| 3a | `f630e225` | Private parameterless ctor on class-backed enums; changelog entry |
| 3c | `bdd13b53` | `HasValue`/`Value`/`TryGetValue`, `_hasValue` writes, `can_carry_payload` |
| — | `d477f843` | `WarningsAsErrors=CS0169;CS0414` gate in `Directory.Build.props` |
| — | `98f7ffd7` | Enum gate consolidated onto `union_names::data_enum`; `DataEnum::is_union_projected()` replaces five inline predicates |
| — | `aa2d550d` | **Case constructors** — the union creation members. Unnumbered in the plan, and the actual gate for 3d |
| 3d | `da741fa9` | `[Union]` and `IUnion` on union-projected enums |
| — | `02c12b35` | Docs: 3d recorded done, its gate corrected, the `IUnion` deferral reversed, 3e marked satisfied-unverified |
| — | `1429746b` | **Projection pass, step one** — `is_managed_only` moved from a render-time local in `body.rs` into `model::…::info::projection` |

**Two corrections from 2026-08-27, both from things that were wrong in this document.** 3d's gate
was recorded as 3c; `[Union]` on a type with no public single-parameter constructor is `CS9385`,
so the real gate was the case constructors, and no item covered them. And the 3d row deferred
`IUnion` on the grounds that its namespace is unspecified — that read the proposal's open
questions as live, when the feature has shipped and `IUnion` resolves from the framework.

**`Issues.md` is diverged three ways and you must reconcile it before trusting it.** The list
below is pinned to `d477f843`, several commits behind head; the file is **modified and unstaged**
in the working tree; and some of its content is corrected here rather than there. Reconcile in
that order — commit or discard the working-tree change first, then re-read, then fix the content.

Open — `2a6da76a`, `4e9a17c3`, `7c8cb22e`, `79be256e`, `b4e07f12`, `5d1ae4c7`. Closed —
`09b82d44`, `ccb105a2`, `1383b84b`, `31248473`, `c33b9cf5`, `e235bc7d`, `8f4c1e2a`.

**Uncommitted working-tree state you will find.** None of it went through a transaction, because
`Issues.md` and repo-root files cannot be scoped into a FileMcp one:

- **`Issues.md` — modified, unstaged.** `79be256e` gained a marshalling-mode sub-question and a
  note that option B landed. Written via `FileMcp:update_issue`, which edits the working tree
  directly.
- **`baseline-nextest.txt` — staged, uncommitted.** Carries a placeholder header
  (`# at <commit>, <date>, <machine>`) never filled in. Fill it or drop the file; a pass count
  with no command and no commit is the defect the header exists to prevent.
- **`.gitignore` — still no `build.txt` entry.**

**Still wrong in `Issues.md`, corrected here but not there:** `5d1ae4c7`'s site count and its claim
that nothing calls the helper — see the counting table below — and `79be256e`'s site-count
paragraph, which says "not reconciled here" and now is.

### Four things worth knowing before you start

**1. `5d1ae4c7` is consolidated, and its own count is wrong.** Every enum output pass used to open
with the same inline `match type_kind { DataEnum(e) => e, Result(_, _, e) => e, Option(_, e) => e,
_ => continue }`. `98f7ffd7` replaced that with
`model::common::types::union_names::data_enum(type_kind)`, and replaced five inline copies of
`variants.iter().any(|v| v.can_carry_payload)` with `DataEnum::is_union_projected()`.

The counting, once, so it is not restated anywhere else in this file:

| Figure | Meaning |
|---|---|
| **10** | sites that *bind* the payload and were rewritten by `98f7ffd7` |
| **12** | those ten plus `all.rs` and `body.rs`, which match for effect and discard the binding — left alone deliberately |
| **~14** | those twelve plus two further re-derivations *inside* `body.rs` — `has_wire_only_payload` and `disposable_variants`, the latter returning `&[Variant]` and so untouched by a literal replacement |

`5d1ae4c7` says **ten** and means the twelve; it also says nothing calls the helper, when
`output/common/wire/cs_names.rs` already did, with two tests asserting the convention. Both
corrections are here and not in the issue.

**`is_managed_only` has since moved into the model** — `1429746b`, see §9 — so the statement in
`5d1ae4c7` that it is computed inline in `body.rs` is also now stale.

**2. `Open items #1` still gates 4b and is still unmeasured.** Positions (a)
`InvalidOperationException` and (c) `?? default` were never reproduced; only (b) was, and it
showed the NRE fires *before* any marshaller, at
`output/common/types/composites/body_as_unmanaged.rs:47` — the **composites** one; an identically
named file exists under `enums/`. `b4e07f12` records the present-tense defect. Reproduce (a) and
(c) before choosing, and do not close `b4e07f12` first.

**3. `ResultVoidVoid` regained union machinery in 3c, deliberately.** If you see it carrying four
empty case types and wonder whether that is bloat: it is the eligibility predicate asking *can a
variant carry a payload* rather than *does one*. See §8.

**4. Do not run the suite before committing, and read the plugin failures rather than the
snapshots.** Validation runs the suite anyway; running it yourself holds the plugin DLLs and is
what caused three `PermissionDenied` validation failures. See § MCP tooling notes, 2026-08-27 for
the full set of traps. And note which layer catches what: when 3d first landed without case
constructors, **`reference_project::interop` passed on an already-accepted snapshot while fourteen
plugin tests failed with `CS9385`.** Snapshots prove text; only the plugin build proves the
generated C# compiles.

### Snapshot workflow

Accept **per step**, not deferred. The accept must run in the *transaction worktree*, not the real
checkout — `ccb105a2`'s "real checkout only" rule applied to LFS stubs, and LFS is gone since
`1383b84b`.

**The incantation lives in §7**, under Environment and workflow, along with why `--no-fail-fast`
matters. It is not repeated here.

**Acceptance is iterative.** `insta` stops at the *first* failing snapshot within a test and some
tests write four; see § MCP tooling notes, 2026-08-27.

**Delete stray build logs before committing.** A `dotnet build > build.txt` left in the worktree
was picked up by a commit and blocked the checkout with *"Untracked working tree file … would be
overwritten by merge"*. The commit itself had already validated; only the checkout failed.

---

## 2. Scope, and what this is not

**Read `docs/csharp-unions.md` § "Two layers, two rules" before writing any code.** It is that
file's first `##` heading, immediately after the untitled preamble, and it is the thing most
likely to be misread.

The short version: this work governs the **generated layer** only.

**The rule stated here previously was the superseded one** — that every `DataEnum` goes through
the union machinery, unit-only ones included, because the generator cannot distinguish a scalar
choice from a payload alternative. That argument covers **domain** eligibility and still governs
everything else: the generator does not guess whether a payload-carrying enum is "really" a
scalar choice. It does not cover **mechanical** eligibility, and that distinction is settled.

**The rule in force: a `DataEnum` with no payload-carrying variant does not receive union
machinery.** No case types, no `Value`, no `HasValue`, no `TryGetValue`. Whether any variant can
carry a payload is not domain knowledge — it is `VariantKind::Tuple` versus `Unit`, visible in the
inventory. Excluded enums keep their current representation: struct, `Unmanaged` mirror and
marshaller all stay, and this rule only declines to *add* union machinery on top. The predicate,
and why it asks *can* rather than *does*, are in §5.

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
practice, but never constrains the projection. `csharp-unions.md`'s preamble states this.

Do not add a **domain** eligibility gate to interoptopus to try to enforce the consumer-layer
rule. It cannot know enough to apply it. The mechanical gate above is a different thing and is
already in force.

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

### The conventions that outlive it

**Templates must not re-sanitize.** `union_names` guarantees uniqueness over the exact strings
it produces. Any casing or escaping applied downstream breaks that guarantee. The `wire`
under-sanitizing described above was an instance of the same invariant broken from the other
direction, and it is fixed — but the rule is what matters, not the instance.

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

## 5. Next: 4, then 4a

**3d and 3e are no longer next.** `[Union]` and `IUnion` landed in `da741fa9`; the case
constructors that 3e's claim rests on landed in `aa2d550d`. What remains of 3e is a fixture that
compiles an implicit conversion — there is nothing left to emit.

**This section covers Track A only — the plan's next row, which is not the objective.** Item 4
(`ToUnmanaged`/`AsUnmanaged` empty guard, no gate), then 4a. **If you arrived here from a pointer
saying "the front is 4", read §0's two-track table first**: plain C# enums are the thing that was
asked for and is missing, and that work is Track B in §9. Item 4 is correct if you are working the
plan in dependency order; it is not what anyone is waiting for.

**4a is now unblocked in a way it was not before.** It constructs via case constructors, and those
exist as of `aa2d550d`. It also deletes the stopgap 3c left behind — `_managed._hasValue = true;`
immediately after the `_variant` copy in `ToManaged` — because 4a's validated switch establishes
the flag implicitly. Delete the stopgap rather than keeping both.

The rest of this section is the record of how 3d and 3e were decided, kept because the reasoning
is load-bearing for Step 4, not because the work is pending.

**3d — `[Union]` and `IUnion`, landed `da741fa9`.** Both are gated on `is_union_projected` and
placed *outside* the `is_managed_only` guard: that guard governs the `Unmanaged` mirror and the
marshaller, and a managed-only enum can still be a union — `DataEnum` in the reference project is
exactly that case, with `[Union]` and no `[NativeMarshalling]`. `IUnion` joins the existing
interface list ahead of `IResult` and `IDisposable`.

**The first attempt at 3d was rolled back, and the reason matters.** `[Union]` on a type with no
*union creation member* is `CS9385`, and none existed: 3b emitted the case types, 3c emitted
`Value`/`HasValue`/`TryGetValue`, and nothing emitted the constructors. The plan recorded 3d's
gate as 3c, which was wrong, and had no item for the constructors at all. **Fourteen plugin tests
caught it** — across the twelve plugin fixtures, several of which have both a `define_plugin` and
a `load_plugin` test. Meanwhile `reference_project::interop`, the snapshot covering every
generated type, **passed**, because its snapshot had already been accepted. The snapshot went
green on output that did not compile.

**Preview-only syntax is settled, not an open question.** `LangVersion=preview` is set in
`Directory.Build.props` and landed with R in `9d664613`, and a .NET 11 preview SDK is already
required to run the tests at all. The CS8652 note is therefore a fact about *consumers* compiling
on stable, not a decision this repository still has to take — which is why it is not in §6.

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

---

## 6. Open decisions

**Only one thing here is actually undecided.** The two entries that used to sit alongside it were
settled and are recorded where the work is tracked, not here — listing a settled call as open
invites someone to re-litigate it.

**Null reaching the marshaller for a class union — undecided.** Today it is a
`NullReferenceException`; the alternatives are a deliberate `ArgumentNullException` or an
`InteropException`. This is `Open items #1` in `csharp-unions.md`, it gates item 4b, and two of
its three positions have never been reproduced. See §1.

**Settled, listed here only so you do not go looking:**

- **The exception split** — `InvalidOperationException` for a default struct union versus
  `InteropException` for a corrupt native tag — is **decided**. `csharp-unions.md` § Step 4 says
  so, and the Todo table's 4b row reads "decided in Step 4; implementation only." Route it through
  `ExceptionForVariant()` so there is one helper. Not implemented.
- **`ToString()` on empty returns `<empty>`.** Decided, not implemented, and **not tracked by any
  row of the Remaining table** — so it is untracked rather than open. Add a row for it or drop it.

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
`CS0246` rather than producing a stale DLL — see § MCP tooling notes, 2026-08-26 for the measured
first-run failure rate. `dll_path_for` is now `dll_path_for::<P>`, and every path to a staged DLL
generates first, so arrival order no longer matters. Two things to keep in mind if you touch it:
the snapshot assertion stays in `define_plugin!` (moving it into the shared helper would make
every loader assert a snapshot it did not ask for), and writes go through
`Multibuf::write_buffers_to_if_changed`, not `write_buffers_to`. The latter rewrites
unconditionally, which bumps mtime, which forces a rebuild, which makes the built DLL newer than
the staged one and re-fires the copy that `stage_is_current` exists to avoid. The content check
also has to live in `Multibuf` rather than in the harness, because the per-buffer `Overwrite`
policy is private and a loop over `iter()` would silently clobber `Overwrite::Never` files.

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

**MCP tool quirks worth knowing.** `FileMcp:write_file` HTML-escapes XML content — `<Project>`
became `&lt;Project&gt;` and had to be repaired. `replace_markdown_section` drops a trailing `---`
separator unless you include it in the replacement. Root aliases are per server: FileMcp's `$N`
and the Rust editor's `$N` are different registries, so a transaction worktree alias from one
cannot be resolved by the other. For `str_replace`'s parser-awareness and the hash guard, see
§ MCP tooling notes, 2026-08-27 and 2026-08-26 respectively — both are stated once, there.

**Path names are not consistent between code and templates, and not even within templates.**
Code uses plural throughout: `output/common/types/{enums, composites, delegates}/`. Templates use
`templates/common/types/{enums, composite, delegate}/` — **`enums` plural, `composite` and
`delegate` singular, in the same directory**. Verified by listing, 2026-08-27. Do not infer one
from the other; list the directory.

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

**`add_markdown_section` works.** An older note here claimed it returns "No approval received";
that was wrong and the claim has been deleted rather than left to be read first.

Two quirks in `replace_markdown_section`: pass the heading as a **leaf**, not a full path, and put
the `##` heading inside `content` — the `title` parameter deletes the old heading without writing
a new one, orphaning the section under its predecessor. Both tools take a dry run first, which
returns the hash to pass back as `expectedTransformedHash`. **The hash is bound to the exact text
you dry-ran**; change a word and you must dry-run again rather than reusing or guessing a hash.

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

Delete `build.txt` afterwards — see § Snapshot workflow for what happens if you do not.

**`search_and_replace` with a bare `file_glob` matches every file of that name.** `body_unmanaged.rs`
and `mod.rs` each exist under both `enums/` and `composites/`; a batch aimed at one silently edited
the other. Anchor on text unique to the intended file, and read the dry run's file list before
applying.

**Plugin DLL staging no longer overwrites a published path.** The earlier first-run
`PermissionDenied` failure came from all processes publishing a logical plugin to one mutable
`_plugins/<name>` pathname. Staging is now content-addressed under
`_plugins/by-content/<fingerprint>/<name>`: identical bytes reuse the existing path through a
read-only check, while changed bytes receive a new directory. The original filename is preserved
for managed assembly lookup. The deterministic Windows regression holds the old path without write
sharing, proves an overwrite fails, and stages the changed bytes elsewhere. See
[`csharp-plugin-staging.md`](csharp-plugin-staging.md).

A first run should now pass. Treat a new `Access denied` as a defect to investigate rather than a
reason to rerun; plugin-preparation filesystem errors now name the operation and paths.

### MCP tooling notes, 2026-08-27

**Do not blindly retry a call that timed out.** The MCP connector can give up while the cargo
process keeps running. Immutable plugin staging removes the loaded-plugin overwrite, but
overlapping cargo processes can still contend for Rust compiler/linker outputs or other fixed-path
test artifacts. Poll for the existing result before re-issuing the command.

**Snapshot acceptance is iterative, not one-shot.** `insta` stops at the *first* failing snapshot
within a test, and some tests write several — `reference_plugins::service::define_plugins` writes
at least `-1` through `-4`. Accept, re-run, repeat until a run comes back clean. Accepting
manually means stripping the `assertion_line: N` header line that `insta` puts in `.snap.new` and
that an accepted `.snap` must not have, then renaming over the `.snap`. A regex over
`assertion_line: \d+\n` across `*.snap.new` handles a whole batch.

**Letting validation find the snapshots is faster than iterating locally.** It runs the full suite
without fail-fast interference and writes every `.snap.new` in one pass.

**`RustEditor:str_replace` is parser-aware and splits matches into `code`, `comments` and
`literals` alternatives.** A pattern that *spans* a comment/code boundary matches **nothing** —
it belongs to no single alternative. Anchor patterns on code only and put any new comment text in
the replacement. A pattern beginning with `// …` will silently return zero matches.

**`RustEditor` needs `root_select` per session.** A fresh transaction worktree comes back as `$N`,
but `str_replace` fails with `Cargo package ownership is unknown` until the root is selected, and
sometimes still fails for a specific file. `RustEditor:search_and_replace` goes through
editor-core rather than the Cargo-aware path and works when `str_replace` will not; it takes a
regex and a `file_glob`, and `expected_files` for a strict apply. **But its replacement does not
honour `\n`**, so a multi-line insert through it produces one long line. For multi-line content
use `str_replace` with a pure-code pattern, or `FileMcp:write_file` for the whole file.

**Repo-root files cannot be scoped into a FileMcp transaction.** `folders: ["docs", "."]`,
`folders: [""]` and a `file:`-only transaction are all rejected; passing `file:` alongside
`folders: ["docs"]` lets you *write* the root file in the worktree but the commit gate rejects it
as `outOfScopePaths`. `Issues.md` therefore has to be edited with `FileMcp:update_issue` against
the real working tree, which leaves it dirty for a human to commit.

**`FileMcp:update_issue` replaces the entire description.** Re-emit the whole body with your one
change; the dry run's diff is the check that nothing else moved. It caught an accidental deletion
of two paragraphs on the first attempt.

**The counting is unreliable and it is not you.** `cargo nextest run -p interoptopus_csharp`
reported **77** locally and **78** under Guarded validation, same worktree, minutes apart. See §1.

---

### MCP tooling notes, 2026-08-28

**`file_glob` matches by file *name*, across everything under the root — this is the contract, not
a defect.** An earlier version of this note called it a bug; the parameter is documented
`/// File-name glob; a path separator is rejected`, and the `file`/`path` argument is the *search
root*, not a scope. Scoping to one directory was never on offer.

It still surprises. `body.cs` matches both `templates/common/types/enums/` and
`templates/common/types/composite/`; `body_to_unmanaged.cs` likewise; `mod.rs` matched **fifty
files**. Passing `path` as the exact file does not narrow it — the tool still reports
`expected_files must exactly match final changed files, missing [the sibling]`. Disambiguate in
the **pattern**, by including a line unique to the intended file, or write the file whole with
`FileMcp:write_file`. The dry run catches this every time; it is only dangerous if skipped.

**A `file_glob` containing `%23` matches no file — and that is a correct answer to the wrong
question.** `insta` snapshots are named `r#mod__…snap`. URL-encoding the `#` produces a *valid*
file-name glob that simply matches nothing, so there is no error to raise. Three separate
conclusions were drawn from the resulting empty list before it was noticed, including reporting a
commit as absent from the repository when it was `HEAD`. Use `*.snap` and filter by pattern.

The real gap was that an empty result could not be told apart from a glob that matched no files.
`search_and_replace` reports `files_searched`; `search_regex` did not, so both cases returned an
identical empty `matches` array. A `files_searched` field was added to `SearchRegexResult` in
`rust-mcp-transform` for exactly this: **zero means look at the glob, non-zero with
`total_matches: 0` means the pattern is genuinely absent.**

**Line endings, and this one cost more than any trap above.** `interoptopus` is LF;
`rust-mcp-transform` is **CRLF**. A multi-line pattern written with `\n` silently matches nothing
in a CRLF repository — six consecutive edit attempts returned zero matches before the cause was
found. Use `\r?\n` in any multi-line regex that might run outside this repo.

The diagnostic signature matters: if `str_replace` returns zero matches on a pattern containing
**no comments and no string literals**, the category rule below is not the explanation — suspect
line endings, or a root that resolves to a different Cargo workspace.

**`search_and_replace` cannot emit newlines**, restated because it was reached for reflexively
four times on multi-line C#. A `\n` in the replacement lands as the two characters `\` and `n`; a
multi-line replacement collapses onto one line and stays *syntactically valid*, which is why it
survives review. For multi-line edits to a `.cs` file, `FileMcp:write_file` is the only reliable
route — `RustEditor:str_replace` is Rust-parser-aware and cannot parse C#.

**`str_replace` patterns must lie entirely within one category.** The 08-27 note covers
comment/code; the same applies to **string literals**. A pattern containing `"…"` — an
`assert!(cs.contains("struct Flag"), …)`, a `context.insert("name", name);` — matches nothing.
Anchor on pure code such as `let mut context = Context::new();` and put the literal in the
*replacement*, which has no such restriction.

**Appending to end-of-file:** `search_and_replace` with the pattern `\z` works, but only for a
single line. For multi-line, append a valid marker item — `const MARKER: () = ();` — then
`str_replace` the marker, whose identifier is pure code.

**Offset arithmetic on table cells is not worth it.** Computing a status cell's offset from its
paragraph's offset was wrong twice — once merging two rows, once leaving a stray `n`. Get fresh
offsets from `find_markdown_elements` after every applied edit, and read the dry-run diff rather
than trusting the arithmetic.

#### The pattern behind four separate traps

Four times, a build artefact drifted from its source because something built it out of band, and
each time the symptom looked like a generator defect:

- **`templates.tar`** — templates are embedded via `include_bytes!(concat!(env!("OUT_DIR"), …))`,
  and `build.rs` emitted one `cargo:rerun-if-changed=templates/`. Cargo does **not** recurse for
  that directive, and every template is nested, so editing one never repacked the tarball. Fixed
  in `fcad90c`; a worktree created before it needs one touch of a template file to recover.
- **`Bindings/Interop.cs`** — gitignored, written only when `reference_project::interop` runs.
  Validation runs in the transaction worktree, so the main checkout's copy is permanently stale.
  Reviewing it produced three wrong diagnoses in a row.
- **`.msbuildcache/`** — untracked, regenerated by every C# build, and blocks `git checkout` of a
  candidate tree, which is how a `docs`-only commit failed with `1 conflict prevents checkout`.
- **Plugin DLLs** — the `Justfile` records this one as solved history: an out-of-band
  `build-dotnet-plugins` step let the DLLs drift from the interop sources and surfaced as
  `ApiMismatch`. The fix was deleting the step so `define_plugin!` builds during the test run.

The shape is always the same: something is generated, something else caches it, and the cache
invalidation is wrong or absent. **When generated output disagrees with the source you are
reading, suspect the cache before the generator.** Three of the four cost more than an hour each,
and in every case the source on disk was correct.

#### The `Justfile` is the maintained interface

`CLAUDE.md` was deleted in `0a4990c` — auto-loaded by Claude Code, unused here, and its
architecture section had drifted into listing files that no longer exist. The `Justfile` at the
repo root is upstream, current, and what CI runs: `.github/workflows/rust.yml` invokes
`just binstall-deps --force` then `just ci --verbose`.

`just ci` is `build` → `test` → `test-dotnet` → `lint`, and **`lint` covers ground the transaction
gate may not**: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo doc` with
`RUSTDOCFLAGS="-D warnings"`, and `diff -q crates/core/README.md README.md`. `test` also runs
`cargo test --doc`, which nextest does not. Confirm what the gate actually covers rather than
assuming — "both steps passed" is a narrower claim than it sounds, and it was asserted wrongly
here more than once.

`just update-snapshots` is `INSTA_UPDATE=always TRYBUILD=overwrite cargo nextest run --all-features`
— it replaces the strip-`assertion_line`-and-rename ritual described above, which was performed
by hand perhaps fifteen times before anyone read the `Justfile`.

`just test-agent` exists upstream and its own comment invites agents to edit it for the task at
hand. It is the sanctioned place for task-specific test wiring; the other recipes enforce project
conventions and should be left alone.
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

---

## 9. The projection pass — step one of three landed

**Why this exists.** The model has no notion of *how a type is projected*. It knows what kind a
type is (`struct_class`, `managed_conversion`, `disposable`), but not whether it should get union
machinery, plain-enum treatment, or an `Unmanaged` mirror. That decision is currently emergent —
it falls out of which output passes happen to fire — which is why you cannot say "emit this enum
as a plain C# `enum`" without editing four passes that never ask. `Issues.md` `5d1ae4c7` names
this as a missing "third category" and deliberately declines to design it.

**Step one landed in `1429746b`.** `model::common::types::info::projection` now answers two
questions that were previously computed inline in `output/…/enums/body.rs` at render time and
shared with nobody:

- `crosses_ffi(ty) -> Option<bool>` — does the type need an `Unmanaged` mirror and a marshaller.
  This is the old `is_managed_only`, inverted.
- `has_wire_only_payload(ty) -> Option<bool>` — is it a `DataEnum` with a `WireOnly` payload.

**They are two values, not one, and conflating them is a real bug.** `crosses_ffi` is false for a
`WireOnly` payload *and* for a `Result`/`Option` whose `Ok` is a `Service`. But only the former
also forces the type non-disposable: a wire-only payload is GC-managed, whereas a Service-backed
`Result` still owns a native resource. Substituting one for the other silently makes
Service-backed results non-disposable. That was caught during step one, in a step whose entire
purpose was to change nothing.

**This pass cannot use the write-once idiom the other info passes use.** `struct_class` and
`disposable` may skip a type they have already answered, because their input `managed_conversion`
returns `None` while a type is still resolving — a genuine not-ready signal. `projection` reads
raw `TypeKind`s instead, and a kind is *always* something. There is no not-ready signal, so an
answer cached early would go stale when a later kind pass reclassifies a payload. It **recomputes
each round and reports `changed` only on difference**, which converges under the same fixed-point
contract. The module comment says this; do not "optimise" it into a `contains_key` skip.

**Accessors return `Option<bool>`, like `disposable`, not bare `bool` like `struct_class`.** This
value decides whether machinery is emitted at all, so collapsing absent into "no" would silently
drop a type's marshalling.

### Step two — add `shape`, still a no-op

Add a shape field with two values, union and struct, where union is exactly today's
`DataEnum::is_union_projected()`. Move the call sites that read that predicate onto the new
pass. **Snapshots must not move**; if they do, something else was wrong.

**There are seven call sites, not five — counted by regex, 2026-08-27.** An earlier version of
this line said five, which is the figure in `DataEnum::is_union_projected`'s own doc comment,
and that one describes the *pre-consolidation* state rather than today's callers. The seven live
in six files: `definition.rs:55`, `body.rs:125`, `body_case_types.rs:57`,
`body_union_members.rs:57`, `body_unmanaged.rs:62`, and `body_ctors.rs` at **both** 66 and 76.

**Three of those seven are not the same question and must not be collapsed.**
`body_ctors.rs:66` and `body_unmanaged.rs:62` read
`struct_class.is_struct(*type_id) && data_enum.is_union_projected()` — that is `writes_has_value`,
which asks whether the type is *struct-backed* **and** union-projected. Only the second half moves
onto the shape pass; the first half stays with `struct_class`. `body_ctors.rs:76` is a separate,
plain read in the same function, which is why that file contributes two.

Keep shape and `crosses_ffi` **orthogonal**. They are genuinely independent: `DataEnum` in the
reference project is managed-only *and* union-projected — it gets `[Union]` and no
`[NativeMarshalling]`. That is exactly why 3d's attribute had to sit outside the `is_managed_only`
guard, and why a single flat enum of projections would lose the case.

### Step three — add plain-enum, the first step that changes output

This is option C in `Issues.md` `79be256e`: unit-only enums become
`public enum Color : byte { Red, Green, Blue }` instead of a ~120-line struct. (`Issues.md` calls
the whole idea "option C"; the two implementation routes below are deliberately not lettered, so
they cannot be confused with it.)

**Read this before planning it — an earlier version of this section described the wrong mechanism
twice.** It first said four unmanaged passes would "decline by construction." It then offered a
choice between two routes, one of which cannot work at all. Both corrections come from reading
`all.rs`, `all.cs`, `definition.rs`, `definition.cs` and `body.rs`, 2026-08-27.

**There are two emitters, not twelve.** `all.rs` renders `templates/common/types/enums/all.cs`
from exactly two inputs — `enum_definition` from `definition.rs` and `enum_body` from `body.rs`.
The other ten passes under `enums/` (`body_case_types`, `body_union_members`, `body_ctors`,
`body_unmanaged`, `body_unmanaged_variant`, `body_to_unmanaged`, `body_as_unmanaged`,
`body_exception_for_variant`, `body_tostring`, `body_from_call`) are **fragments that `body.rs`
composes**, every one of them read with `map_or("", …)` or `unwrap_or(&[])`. Making them decline
does not change the declaration shape; it only empties the body, and an empty fragment is already
the ordinary case.

**A declining emitter deletes the type — and that is true of both emitters, not just the body.**
`all.rs` guards each one the same way:

```rust
let Some(enum_definition) = enum_ty.get(*type_id) else { continue };
let Some(body) = enum_body.get(*type_id) else { continue };
```

Whatever step three does, it must leave a rendered value present for every type.
Present-but-empty is fine; absent silently removes the type from the generated output.

**The generated type is two `partial` declarations, which rules out the obvious route.**
`definition.cs` emits `{{ visibility }} partial {{ struct_or_class }} {{ name }}` and closes its
own braces around `_variant`, `_hasValue` and the payload fields; `body.cs` emits a second
`partial` declaration carrying the members; `all.cs` is nothing but `{{ enum_definition }}` and
`{{ enum_body }}` concatenated, with no braces of its own. **C# `partial` is valid on class,
struct, interface and record — not on `enum`.** So "branch both emitters, one producing the
`enum` declaration and the other an empty body" is not a stylistic choice that lost: a C# enum
cannot be split across two declarations, and making `body.rs` return nothing instead trips the
guard above and deletes the type.

**Before any of that: step three is not primarily an emission change, and the routes below are
downstream of the decision that actually matters.** `managed_conversion.rs` classifies every
`DataEnum` as *at least* `To` — its comment says so in as many words, "Enums: at least To; Into
if any variant data is Into" — and never `AsIs`. That one classification is what produces all
three of the downstream breakages found on 2026-08-27, which are not three problems:

- **Composites.** `NestedArray`'s generated `Unmanaged` calls `field_enum.ToUnmanaged()`, while
  `field_bool` and `field_int` beside it are copied straight across. The difference is nothing
  but `ManagedConversion`.
- **Slices.** `slices.rs` sends `AsIs` elements to `fast.cs` and everything else to
  `marshalling.cs`. Same classification, same reason.
- **The mirror itself.** A type with a non-`AsIs` conversion is what an `Unmanaged` nested struct
  and a marshaller exist to serve.

**So the lever is making a plain-enum-projected `DataEnum` `AsIs`,** and the emission change
follows from it rather than the other way round. Get that right and composites copy the field
like a `bool`, slices move to the pinning path, and `unmanaged_names` stops manufacturing
`X.Unmanaged` — all without touching those emitters. Get it wrong and no amount of work in
`definition.rs` will help.

**The pass dependency, read rather than reasoned about — an earlier version of this paragraph
called it a cycle and told you to check convergence. It is not a cycle.** `projection::process`
takes only `type_all`; `managed_conversion::process` takes only `type_all`. Neither reads the
other. Threading `projection` into `managed_conversion` adds one straight edge and there is
nothing to converge.

**The real hazard is the opposite pairing, and it is easy to miss.** `managed_conversion` is
**write-once** — it opens with `if self.managed_conversion.contains_key(cs_id) { continue; }` and
defers on a not-ready input by leaving the key absent (its `pending` flag). `projection`
**recomputes every round**. A write-once consumer reading a recompute producer can cache an
answer the producer later revises, which is precisely the staleness `projection`'s own module
comment exists to prevent — reintroduced one pass downstream.

**For this particular value it happens to be safe, and the reason is worth keeping.**
`is_union_projected()` reads only `can_carry_payload`, which `enum_variants.rs` and `fallback.rs`
set when the variant is constructed and nothing afterwards revises. It performs no lookup into
other types, so it cannot change across rounds. Contrast `wire_only` in the same pass, which
resolves a variant's payload type through `types.get` and *does* settle over rounds — that half
is what the recompute idiom is actually for.

**So the requirement is a not-ready guard, not convergence checking.** If
`projection.projection(id)` is `None`, `continue` — exactly the existing `pending` idiom — rather
than falling through to `To`. Without that guard `managed_conversion` can answer for an enum
before `projection` has answered for it, cache `To`, and never revisit, because write-once passes
do not come back.

**Two routes remain. Try the first.**

- **`definition.rs` emits the whole `enum`, and `body.rs` yields an empty string.** Both guards
  pass, `all.cs` renders the enum followed by blank lines, and neither `all.rs` nor `all.cs`
  changes. Smallest edit, and the only one that leaves the shared renderer alone.
- **Route at `all.rs`** — detect the plain-enum projection there and render a different template
  beside `all.cs`, leaving `definition.rs` and `body.rs` untouched for those types. Reach for this
  only if the first route hits something.

**`definition.rs`'s existing `variants` context cannot supply the members, and this is the part
most likely to be missed.** It is built with `filter_map` on `v.ty?` — payload-carrying variants
only. For a unit-only enum that list is **empty**, so `Red = 0, Green = 1, Blue = 2` cannot come
from it. The plain-enum branch needs its own list carrying `stem` and `tag` for *every* variant.

**`variant.name` in `definition.cs` is not a breach of the stem rule.** `definition.rs` does
`m.insert("name", v.stem.clone())` — the template key is `name`, the value is the stem. §3's rule
is intact here; do not "fix" it.

**Carriers can never reach plain-enum, and this is enforced, not incidental.**
`fallback.rs::payload_variant` sets `can_carry_payload: true` **unconditionally**, with a doc
comment giving the reason: `Result<(), ()>` still declares `Ok(T)`/`Err(E)`, and erasing that is
exactly what stripped `ResultVoidVoid` of its projection once already. `unit_variant` is used only
for `None`, `Panic` and `Null`. Every synthesised `Result`/`Option` carrier gets at least one
`payload_variant` for its `Ok`/`Some` position, so `is_union_projected` is always true for them.
A carrier reaching plain-enum would break `IResult` and `body_from_call`; it cannot.

**The canary fired, and it flipped exactly as predicted.** `pattern_ffi_slice_of_unit_enum`
landed in `4aad161`; step three landed in `ca6aafa`. Before, `SliceEnumDocumented` was a class
holding `IntPtr _data`, allocating through `Marshal.AllocHGlobal` and copying each element via
`Marshal.PtrToStructure<EnumDocumented.Unmanaged>` and `AsUnmanaged()`. After, it holds
**`GCHandle _handle`** — `fast.cs`, the pinning path, alongside `SliceByte`, `SliceUint`,
`SliceInt` and `SliceBool`. The conversion became `AsIs`, the template selection followed, and
nothing in `slices.rs` was touched.

**Do not read the flip itself as proof, and an earlier version of this paragraph did.** It
claimed "all fourteen plugin tests passed, so the generated C# for `SliceEnumDocumented` compiles
and runs." That does not follow. `reference_project::interop` writes bindings to
`tests/reference_project/Bindings` and `benches/dotnet` and asserts a snapshot — **it compiles
nothing**. The plugin fixtures compile plugin inventories, declared by `plugin!`, which do not
contain `pattern_ffi_slice_of_unit_enum`. Two true, unrelated facts in one sentence manufactured
a conclusion.

**It is now answered, by execution.** A pinned `EnumDocumented[]` survives the boundary:
`[A,B,B]` → 2, `[B,A,B,C,B]` → 3, `[A,C,A]` → 0. Three lengths, three arrangements, which no
wrong element stride satisfies jointly. And the risk was overstated from the start — **the enum
never crosses as an enum**; `{ IntPtr, ulong }` does. `GCHandle.Alloc` is the only runtime
component that touches the element type, and an `enum : byte` array pins as a `byte[]` does.

**The condition that let this go unnoticed matters more than the answer, and is still open.**
**Nothing in this repository compiles the generated reference bindings.** The plugin fixtures do
compile generated C# — but only for the *plugin* inventory, so the reference output has no
compiler pointed at it anywhere. That is how text-valid, C#-invalid output survived a green
suite: after step three the generator emitted `IsA`/`IsB`/`IsC` accessors on types now projected
as plain enums, at two sites, and every test passed. It was found only when something finally
compiled the output.

**Be precise about what failed, because "snapshots are weak" is the wrong lesson.** `insta`
compared correctly and the accepted snapshot was accurate. A snapshot is a sound regression guard
*once something has established the output is valid* — here nothing ever did, so it was guarding
a baseline no one had checked. The fix is a compiler downstream of the reference bindings, not
distrust of snapshots. Until that exists, expect the next defect of this shape.

**There are two C# projects in `tests/reference_project/Tests/`, and an earlier version of this
paragraph conflated them.** It named `InteropSpike.csproj` as the harness holding the
hand-written xUnit tests. It is not:

| | `Tests.csproj` | `InteropSpike.csproj` |
|---|---|---|
| Test framework | `xunit.v3.aot` 4.0.0-pre.128 | `TUnit` 1.36.0 |
| Compile items | default (on) | `EnableDefaultCompileItems=false`, one `<Compile>` for `InteropSpike.cs` — **which does not exist** |
| In `ReferenceProject.slnx` | yes | **no** |
| References `Bindings.csproj` | yes | yes |
| Builds the cdylib | no | yes, an `Exec` running `cargo build -p reference_project` |

So the ~30 `Test.*.cs` files — `Test.Core.Enums.cs`, `Test.Core.Arrays.Nested.cs` and the rest —
belong to **`Tests.csproj`** and are compiled by it. `InteropSpike.csproj` is an abandoned spike
in no solution, compiling a source file that was never added. `ReferenceProject.slnx` lists
exactly `Bindings\Bindings.csproj` and `Tests\Tests.csproj`.

**Nothing in the Rust sources references any of them, so `cargo test` never builds them**
(searched 2026-08-27). That remains the gap.

**But compiling them would not answer the question either — and this is the part worth keeping.**
Grepped 2026-08-27: **no file under `Tests/` constructs a slice of anything.** Every
`Slice*.From` / `.Slice(` call in the repository is in `benches/dotnet/Benchmark.cs`, which is
frozen and uses `byte`/`Vec3f32`, never an enum. So the work item is not "repair a project", it is
**write one test**; `Tests.csproj` is only the host. The entry point already exists —
`pattern_ffi_slice_of_unit_enum` counts variants equal to `EnumDocumented::B` and returns the
count, so `[A, B, B]` must return `2`. Content-derived, not an it-did-not-throw test.

**Two practical traps for whoever writes it, and the second is a gap in the repository rather
than a mistake you might make.** The bindings on disk are stale in a way worse than
"pre-plain-enum": `Bindings/Interop.cs` still shows `EnumDocumented` as a struct carrying
**`bool _hasValue`**, which item 3c's scoping removed from unit-only enums, so they predate that
too. Regenerate first, in a transaction — `reference_project::interop` rewrites tracked `.cs`
files and a `.snap`.

**Nothing provisions the cdylib, and this is worth its own fix.** `Bindings.csproj` copies
`target/debug/*reference_project*` through a `Content` glob — **the glob is correct**; a
suspicion that it pointed at the wrong directory was checked on 2026-08-27 and was wrong.
`target/debug` is exactly where `cargo build -p reference_project` puts a cdylib. The problem is
that nobody runs that: `cargo test` builds the crate as a *dependency*, leaving the unhashed DLL
in `target/debug/deps` with no hardlink up a level, and `Tests.csproj` has no cargo step at all.
Only `InteropSpike.csproj` ever ran one, in a `BeforeTargets` hook.

**So `DllNotFoundException` is the default outcome for anyone running that suite** — and it is
worth naming beside the four-outcome triage above, because three of those four are code faults
and this one is not. Diagnose it as provisioning before suspecting marshalling. A Rust test that
shells out to `cargo build -p reference_project` is the better home for this than a
`BeforeTargets` hook: it puts provisioning where `cargo test` will actually trigger it, and it
avoids the cargo-inside-MSBuild-inside-cargo nesting flagged above. **Lift that step somewhere
before deleting `InteropSpike.csproj`**, which is currently the only record of how the native
library gets provisioned.

**The plugin-fixture alternative is struck, not merely doubted.** Adding a slice-of-enum case to
a plugin fixture *cannot* answer the pinning question: `plugin!` declares methods that **Rust
calls into C#**, and pinning happens only when **C# builds a slice from a managed array and
passes it to Rust**. That direction does not exist in a plugin. The reference-project bindings
are the only place it does. Anyone who reaches for the cheap route here loses a day and learns
nothing; that is why this section is about a C# project at all.

**Read `slices.rs` before reasoning about this — the first version of these paragraphs got the
mechanism backwards.** The split is not blittability and the emitter never tests for it. It is
the element's **`ManagedConversion`**: `AsIs` renders `rust/pattern/slice/fast.cs`, everything
else renders `rust/pattern/slice/marshalling.cs`. The module comment says so in as many words —
only `AsIs` elements can be projected directly over native memory. An enum has an `Unmanaged`
mirror, so its conversion is not `AsIs`, and that is the whole reason it landed on
`marshalling.cs`. Nothing about the CLR's notion of blittable enters into it.

**And the enum is not on a special path.** `SliceVec3f32` is emitted identically — same `class`,
same `[NativeMarshalling]`, same `AllocHGlobal` and per-element copy — even though `Vec3f32` is
a plainly blittable struct. `AsIs` is much narrower than blittable. The other side of the split
is visible in `SliceByte`, the `Slice<u8>` binding, which takes `fast.cs` and holds a
**`GCHandle`**: it pins the managed array rather than copying it.

**That inverts the conclusion, and makes the canary more valuable than "it will lose members".**
Once a unit-only enum is emitted as a plain C# `enum` it has no `Unmanaged` mirror, so its
conversion should become `AsIs` — and the slice flips from `marshalling.cs` to `fast.cs`, onto
the `GCHandle` **pinning** path. Pinning an enum array under the classic marshaller is precisely
the hazard §9 has been circling. So the blittability question is **not** moot; step three is the
moment it becomes live, by a route the earlier text had exactly reversed.

Either way the canary fires, and the two outcomes are worth telling apart when it does. If the
conversion becomes `AsIs`, the slice silently changes strategy to pinning and any failure is at
**runtime**. If it does not, the slice stays on `marshalling.cs` and needs
`EnumDocumented.Unmanaged`, `AsUnmanaged()` and `ToManaged()`, none of which will exist — a
**compile** error. Check which happened before concluding anything.

One unrelated defect visible in that output, pre-existing and not caused by the canary:
`marshalling.cs` writes `}public partial class Slice…` with no newline between the closing brace
and the following `partial`. It affects every marshalling-path slice, not just the new one. It
compiles; it is merely ugly.

Open questions `79be256e` lists, plus one this work added:

- What `Unmanaged` contains for a unit-only enum — **read, 2026-08-27, and it is a green light on
  layout.** `body_unmanaged.rs` builds its `variants` list with `filter_map` on `v.ty?`, so for a
  unit-only enum the list is empty and `body_unmanaged.cs` renders nothing but
  `[FieldOffset(0)] internal {discriminant} _variant;` plus a `ToManaged` that copies the tag and
  returns. `writes_has_value` is `is_struct && is_union_projected`, so it is false and no
  `_hasValue` write appears either. **The mirror is layout-identical to the discriminant primitive
  itself**, which means replacing struct, mirror and marshaller with a plain C# `enum` of the same
  underlying type changes nothing the native side can observe. What remains unanswered is the
  marshalling-mode bullet below, which is about blittability, not layout.
- How Rust `#[repr]` maps to the C# underlying type — **closed structurally, 2026-08-27, and the
  answer is stronger than any empirical check.** `EnumDocumented` and friends carry no `#[repr]`
  in source; the `#[ffi]` macro generates it. In `proc_macros_impl/src/types/emit.rs`,
  `generate_repr` for `TypeData::Enum` always routes to `layout_tokens`, and
  `discriminant.rs::optimal_discriminant` computes **one** `DiscriminantChoice` from which
  `repr_attribute` emits the Rust `#[repr(..)]` and `layout_tokens` emits the C# `Layout`. One
  choice drives both sides, so **divergence is impossible by construction** — not merely absent
  in the current corpus. `A`/`B`/`C` auto-number to a max of 2, hence `#[repr(u8)]` and
  `Layout::Primitive(U8)`, hence `byte`. Note this also makes the `_ => Primitive::Int` arm in
  `kind/enum.rs` **dead for `#[ffi]` enums**: a `#[repr(C)]` fieldless enum would be four bytes
  against a one-byte C# side, but the macro never emits `Layout::C`, so that arm is unreachable
  here. Do not cite it as evidence of a mismatch risk. `EnumNegative` still needs a signed type,
  which the same mechanism supplies.
- **Which marshalling mode the bindings run under — answered by execution, and the question was
  overstated.** `DisableRuntimeMarshalling` is **not** emitted; `templates/rust/header.cs` is a
  ten-line comment banner with no assembly attributes. Earlier revisions of this bullet treated
  that as a live classic-marshaller hazard — "enum arrays and pinning are the failing cases" —
  and it was wrong. **A pinned `EnumDocumented[]` survives the boundary.** Three cases, run:
  `[A,B,B]` → 2, `[B,A,B,C,B]` → 3, `[A,C,A]` → 0. Three lengths and three arrangements, which
  a wrong element stride could not jointly satisfy — confirmation, not an it-did-not-throw pass.

  **The reason it was never really at risk is worth keeping**, because it is the thing the
  earlier framing got backwards: **the enum never crosses as an enum.** What crosses is
  `{ IntPtr, ulong }`. The only runtime component that touches the element type is
  `GCHandle.Alloc`, and an `enum : byte` array pins exactly as a `byte[]` does. There was no
  marshaller decision to get wrong. Treat "classic marshaller cannot handle enum arrays" as
  retired, not merely untested.
- Exhaustiveness is genuinely lost. A `switch` over a plain C# `enum` is never exhaustive, because
  `(Color)99` compiles. Closed enums would have fixed this and **did not ship in C# 15**. A Roslyn
  analyzer is the interim substitute for the compile-time half; there is no runtime half to lose,
  because `Unmanaged::ToManaged` copies the tag blind today and validates nothing.

### Wiring a new model pass — nine sites, two pipelines

Registering `projection` touched: `info/mod.rs`, then a `Config` field, a `Pass` field, a
`Pass::new(…)` line and a `process(…)` call in **each** of `pipeline/rust/library.rs` and
`pipeline/dotnet/library.rs`. Threading it into an output pass adds the parameter plus both
`o.<pass>.process(…)` call sites.

**There are two pipelines and they order these passes differently** — rust runs
`managed_conversion, disposable, nullable, struct_class`; dotnet runs
`managed_conversion, struct_class, disposable, nullable`. It is a convergence loop so order should
not determine the result, but placement is two decisions, not one. **Forgetting the dotnet
pipeline breaks only the plugin fixtures**, which is the slowest failure to notice.