# Rust interop service

Status: **approved design**, 2026-10-17. R1 to R3 and E1 to E3 are done and E5 has its first
increment; progress is tracked in the Todo section below. Open decisions marked "Proposed
(2026-10-24)" await confirmation.

## Goal

Generate interoptopus FFI facades and a public C# API for a Rust crate from its rustdoc
JSON. Scripts call it as `Get<MpsAgent.RustInteropService.IRustInteropService>()`, driven
by a per-repository manifest. The first target is gix (gitoxide fork) -> GixSharp.

## Not in v1

- Generating callbacks, progress, cancellation or IO APIs. These stay hand-written.
- Semantic tests. Only smoke tests are generated.
- Replacing the hand-written, LibGit2.Native-shaped GixSharp API. The generated mirror is
  additive and aimed at scripts.

## Components

| Component | Location |
|---|---|
| Engine (Rust command-line tool) | This repository, `crates/rustdoc_gen` (package `interoptopus_rustdoc_gen_mps`, binary `interoptopus-rustdoc-gen`). Released with interoptopus via `publish_localcargo`; the binary is packaged with `package_rust_binary`. |
| Names sidecar | This repository's C# backend writes the generated type and method names next to `Interop.cs`. |
| C# service | `MerkleCompiler/MpsAgent.RustInteropService`, following `MpsAgent.GitService`: contract and options, an `IScriptAdapter`, an internal implementation. |
| Manifest, runtime and outputs | The target bindings repository (GixSharp). |

## Engine

The engine runs as a subprocess of the service. It reads a JSON request and writes a JSON
response; failures exit non-zero with a structured error.

1. **extract** runs `cargo doc` with rustdoc JSON output
   (`RUSTDOCFLAGS=-Z unstable-options --output-format json`):
   - pinned nightly, `--locked`, `--no-deps`;
   - the crate list is the transitive closure of the root crate in `Cargo.lock`, filtered by
     name prefix;
   - the output cache lives outside the repository, keyed by commit, features, toolchain and
     crate list.
2. **analyze** loads the JSON with `rustdoc-types` (format 60), classifies every public
   function, hashes each signature, and rewrites the manifest items and counts. It records
   the provenance it analyzed: hashes of the rustdoc JSON and of the configuration. The
   service extracts again before generating, so a changed checkout yields different JSON and
   a stale manifest.
3. **generate** emits the Rust facade, builds it and runs interoptopus (producing
   `Interop.cs` and the names sidecar), then emits the public C# layer and smoke tests as a
   deterministic set of files. It refuses a stale manifest, one whose provenance or items
   differ from a fresh analysis of the given JSON and configuration. By default it also
   refuses a manifest that is not ready; a request may ask for partial generation, which
   emits the ready items only and reports the rest as pending. Decisions are checked by
   building the generated facade (see E4).

## Service contract

The lifecycle mirrors CppMcp's interop tools (`configure_interop`, `analyze_interop`,
`get_unresolved_interop_methods`, `resolve_interop_methods`, `generate_interop`).

- `ConfigureAsync(manifestPath, configuration)`: any change invalidates readiness.
- `AnalyzeAsync(manifestPath)`: rewrites only the manifest (items, counts and provenance);
  returns total, automatic, unresolved and excluded counts and whether generation is ready.
- `GetUnresolvedAsync(manifestPath)`: returns the same next batch until the manifest changes.
- `ResolveAsync(manifestPath, decisions)`: each decision is checked against the item's
  signature hash.
- `GenerateAsync(manifestPath, worktreePath)`: refuses a stale manifest and, unless partial
  generation is requested, one that is not ready; writes atomically. The service checks that
  `worktreePath` is a transaction worktree it owns; the engine rejects output folders that are
  absolute or lead outside the worktree.

Options: `EnginePath` (override) and `CacheDirectory`. The calls are long-running, so
scripts should run them asynchronously.

## Manifest

One JSON file per target repository.

- **configuration**: workspace `Cargo.toml`, root crate, crate prefix, features, toolchain;
  interoptopus version and facade crate; output folders for generated Rust and C#; path of
  the hand-written runtime module; managed and native namespaces; library name; type-map
  overrides.
- **items**: item path, signature hash, class, status, reason.
- **decisions**: signature hash plus one kind:
  - `handle`: key type and a Rust snippet that re-creates the handle from the key;
  - `record`: owned fields;
  - `convert`: argument mapping;
  - `exclude`: with a reason;
  - `manual`: implemented by hand in a named module.

## Classification (v1)

- **Automatic**: every type is in the type map; borrowed results only for handle types that
  have a decision; generic arguments only through the conversion table.
- **Needs a decision**: unmapped borrowed results and other generics; iterator and platform
  results until a cursor template exists; `&mut self` methods on `Repository`, which need
  the persistent repository runtime.
- **Excluded**: deprecated items, `doc(hidden)` items, trait-impl methods, and config schema
  keys (by rule).
- An item can have several blockers. Reasons record the first one found (receiver, then
  parameters, then the result), so counts by reason are counts of first blockers; how many
  items an answer unblocks is measured by re-classifying with it applied.

## Default type map

- Primitives map directly; `&str`/`String` -> `string`; `BStr`/`BString`/`[u8]` -> `byte[]`.
- `ObjectId` -> hex string, matching GixSharp today.
- Filesystem paths -> bytes, matching gix-ffi today (still open).
- `Option`, `Vec` and `Result` -> their interoptopus equivalents.
- Errors go through the runtime's error mapper.

## Naming and namespaces

interoptopus names each FFI type by its bare Rust identifier, or by the `name` argument on
`#[ffi]`; module paths never take part. gix has 460 public types but only 186 distinct
names, and 69 names repeat across modules (`Error` x166, `Options` x15, `Outcome` x10,
`Platform` x9), so a flat mirror would collide.

- **Internal layer** (`Interop.cs`, one namespace via `Dispatch::single_file`): stays flat.
  The engine gives every facade type a unique, module-qualified name, such as
  `RemoteFetchOutcome`, through the generated struct name or the `name` argument.
  Nothing in this layer is public API.
- **Public layer** (emitted by the engine): namespaces come from the rustdoc module path and
  types keep their Rust names, e.g. `gix::remote::fetch::Outcome` ->
  `Gix.Remote.Fetch.Outcome`. The root namespace is `Gix`.

interoptopus itself needs no change for this.

## Hand-written runtime (per target)

The generated facade calls a runtime module (configuration `runtime_module`, default
`crate::runtime`) that provides:

- `Error`, an FFI type, and `error(e)`, which maps any error a wrapped call returns (the planned
  error contract, P0b in the GixSharp handoff);
- the repository holder, named like the generated service (a mutex-guarded repository or a
  pool), with `From` for each type mapped to `repository` and one accessor per such type, named
  after it in snake case (`repository()`, `thread_safe_repository()`);
- `path` and `path_out`, `object_id` and `object_id_out`: the conversions whose representation
  is still open;
- later, the cursor template.

Text and bytes convert inline through interoptopus' own types.

## Handles

A handle stands for a Rust object that borrows a runtime handle, such as gix's
`Commit<'repo>`. It cannot cross the boundary itself, so it crosses as a key, and the facade
rebuilds it from the key on each call. Accepting a handle decision is the target author's
promise that rebuilding from the key keeps the behavior callers expect; the engine checks that
the reconstruction and every wrapped call compile, never that they mean the same.

- A per-type answer covers methods taking `&self` only. Methods taking `self` by value or
  `&mut self`, such as builders (gix's `Remote::with_url`) and in-place mutators
  (`Reference::peel_to_id`), produce state the key does not hold, so each needs its own
  decision.
- The key travels with its runtime handle: a handle is the pair (runtime handle, key), and a
  result belongs to its receiver's runtime handle.
- Rebuilding refreshes state. A type the library documents as a snapshot loses that behavior:
  gix's `Reference::delete` fails if the reference changed since it was observed, and
  rebuilding by name replaces the observed target. Where the public API cannot rebuild the
  observed state, such methods stay manual or return a `record`.
- State held only in memory, such as gix's in-memory object overlay (`with_object_memory`),
  lives in one instance and is lost when the facade rebuilds through another; such APIs need
  the persistent runtime holder or stay manual.

Every call rebuilds the object, so bulk reads, such as a 10,000-commit log reading five fields
each (50,000 lookups), should use a per-item `record` override that returns the fields
together.

## Determinism and safety

- Stable ordering; generated file headers record the engine version, rustdoc format version
  and source commit.
- A check that regenerating produces no diff.
- Generated code lives in its own folders and namespace and never touches hand-written
  files.
- Writes to repositories happen only in transactions.
- Machine requirements: rustup with the pinned nightly and MSVC build tools.

## Reference measurements (2026-10-17)

- rustdoc JSON for `gix` and its 59 `gix-*` dependencies with gix-ffi's feature set:
  60 files, format version 60, 108 s with `nightly-2026-07-18`.
- Public `gix` surface: 704 functions (682 methods on public types and 22 free functions),
  excluding 202 re-exports of the plumbing crates.
- First classification (E3, 2026-10-23), with `Repository` and `ThreadSafeRepository` as
  runtime handles and `gix::config::tree` excluded: 712 items (the 704 plus 8 deprecated):
  39 automatic, 75 excluded, 598 needing a decision. First blockers: receiver not a handle 405,
  unmapped type 113, borrowed 28, generic 26, `&mut self` on the repository 13,
  iterator or platform 7, other 6.

## Open decisions

- Object ID representation (hex or binary).
- Filesystem path representation.
- Whether `gix-ffi` moves into GixSharp with its git history.
- Handle decisions per type instead of per item: 405 of the 598 open items report their
  receiver (`Reference`, `Object`, `Commit`, `Tree` and about 20 other types) as their first
  blocker; how many a per-type answer unblocks is measured by re-classifying. Proposed
  (2026-10-24): one answer per type, covering `&self` methods only, with per-item overrides
  (see Handles).
- Proposed (2026-10-24): handles cross the boundary as keys. Every API returning a handle
  type, including inside `Option` and `Vec`, returns its key; every method of a handle type
  becomes a facade function taking the repository and the key; the public C# layer builds the
  handle object (`Gix.Commit` holds its `Repository` and id). interoptopus returns services only
  from constructors and never inside `Option` or `Vec`, so per-type Rust services cannot
  express these results. The repository stays a service because it holds state.
- Proposed (2026-10-24): decision payloads are structured. A `handle` names its type, its key
  mapping and two Rust fragments, `to_key` (such as `{value}.id`) and `from_key` (such as
  `{repo}.find_commit({key})`); a `record` lists field paths, checked against rustdoc; a
  `convert` names a source mapping and one fragment (such as
  `gix::refs::FullName::try_from({value})`); a `manual` names a module; an `exclude` gives a
  reason, the only free text, which generation ignores. Only `{value}`, `{repo}` and `{key}` are
  substituted. E4 checks them by building the generated facade with the target's features, so
  each fragment is compiled together with argument conversion, the wrapped call and result
  conversion; compiler errors map back to decisions through each member's generated span.
- Proposed (2026-10-24): no gix knowledge in the engine, done before E4. Built-in mappings
  cover only std (primitives, `String`/`str`, `Path`/`PathBuf`, `Vec<u8>`, `Option`, `Result`).
  `Mapping` becomes `string`, `bytes`, `path`, `handle` and `custom`; a custom mapping names its
  FFI type and two runtime conversion functions, so gix's `ObjectId` becomes a custom entry in
  its manifest. A bound `AsRef<T>`, `Into<T>`, `Borrow<T>` or `TryInto<T>` is satisfied when `T`
  is mapped, and a custom trait bound (gix's `AsBStr`) when rustdoc shows a mapped type
  implementing it, replacing the hand-written conversion table. Trait implementations only
  suggest that a bound holds; E4's build decides. Categories come from trait implementations
  (`Iterator`, `Fn*`, `Read`/`Write`) instead of type names such as `Platform`.
- Proposed (2026-10-24): `analyze` proposes type-map entries by scanning types, with G3:
  handle candidates (returned by free constructors, many `&self` methods, no lifetime
  parameter, `Send + Sync`), string-like types (`Display` plus `FromStr` or `TryFrom<&str>`) and
  byte-like types (`AsRef<[u8]>` or `Deref` to bytes), ranked by how many blocked items each
  unblocks when the classifier re-runs with it applied; a person accepts them into the
  manifest. Representation and meaning (hex or
  binary ids, paths as bytes or text, live or snapshot keys) stay human decisions. `analyze`
  must then read the dependency crates' rustdoc JSON, which `extract` already produces.

## Todo

### Prerequisites (gitoxide fork, first)

- [x] R1 Merge or drop the three unmerged slices (merge/snapshot, sparse-checkout, read-tree recovery). Integrated on gitoxide `main`: sparse-checkout `19d524c8`, read-tree recovery `219b4788`, merge/snapshot `3d5b6f94` (one shared read-tree publication and recovery path); 13 superseded transactions rolled back.
- [x] R2 Move `gix-ffi` to the next interoptopus release. Released 0.19.0 at `d33b37ae` (local registry commit `430744f0`); `gix-ffi` consumes it at gitoxide `aaaa9516`. `ByteReader` became `Send + Sync` (its source behind a `Mutex`; gix-worktree-stream's `Stream::from_read` now requires `Send`); the managed layer needed no changes.
- [x] R3 Make every export return `ffi::Result` (no plain-value returns). All 84 exports do, at gitoxide `aaaa9516`; 116/116 native and 119/119 managed tests pass.

### Engine (this repository)

- [x] E1 Pick the `rustdoc-types` release whose `FORMAT_VERSION` is 60; create the engine crate with `extract`, `analyze` and `generate` subcommands and JSON input/output. Integrated at `97c9ffbc`: `rustdoc-types` 0.60.0; `analyze` on `gix.json` reports 682 methods and 22 free functions (704).
- [x] E2 `extract`: crate list, plus caching. Integrated at `744975c5`: the crate list comes from `cargo metadata --locked` (normal dependencies sharing the root's prefix), rustdoc runs in a target directory owned by the checkout, and results are cached under a content hash of toolchain, features, `Cargo.lock` and crate sources.
- [x] E3 Model, classifier and signature hashes; `analyze` writes the manifest. Check the result against the 704-function reference count. Done: the non-deprecated gix surface is 704 functions; items carry a canonical signature, its hash, a class and a status (ready, unresolved, resolved, stale, excluded), and `analyze` rewrites the manifest only when it changes.
- [ ] E4 Decision validation: structured payloads, per-type handle answers covering `&self` methods only, and the check that builds the generated facade with the target's features. Needs a buildable runtime first: G2's minimal runtime or a stub of the runtime contract.
- [ ] E5 Rust facade emitter (automatic, handle, convert and record kinds) plus inventory registration. Facade types get unique module-qualified names (see Naming and namespaces). First increment, the automatic kind: `generate` emits one service for all types mapped to the runtime handle (functions returning the handle become constructors, including `&self` methods as constructors taking `&Self`; free functions and the handle's own type win name clashes; conversions between handle types are dropped), its `&self` methods, and free functions named after their module path. Every member returns `ffi::Result`. It refuses a stale manifest (provenance or items differ from a fresh analysis) and, unless the request sets `partial`, one that is not ready; it rejects output folders outside the worktree and writes `mod.rs` into `rust_output` only when it changes. An end-to-end test builds the generated facade for a demo crate and validates its inventory.
- [ ] E6 Names sidecar in the C# backend.
- [ ] E7 Emitter for the public C# layer and smoke tests. Namespaces follow rustdoc module paths (see Naming and namespaces).
- [ ] E8 Release with `publish_localcargo`; package the binary with `package_rust_binary`.

### Service (MerkleCompiler)

- [ ] S1 `MpsAgent.RustInteropService`: contract, options and adapter, following the GitService pattern.
- [ ] S2 Engine invocation: subprocess, cancellation, logs, locating the engine binary.
- [ ] S3 Publish, bring it into ScriptMcp, and smoke-test it from a script via `Get<IRustInteropService>()`.

### GixSharp

- [ ] G1 Create the GixSharp repository and move `gix-ffi` and the bindings there.
- [ ] G2 Runtime: persistent repository holder, error mapper, conversions.
- [ ] G3 Write the gix manifest, run analyze, review the unresolved queue.
- [ ] G4 First generated slice: simple `Repository` methods plus `Id`/`Commit` handles; build and smoke-test.

### Dependencies

- R and E1-E3 can run in parallel.
- G4 needs E5-E7, S1-S2 and G2.