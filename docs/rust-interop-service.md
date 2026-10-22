# Rust interop service

Status: **approved design**, 2026-10-17. E1 and R1 are done; progress is tracked in the
Todo section below.

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
   function, hashes each signature, and writes the manifest items and counts. Decision
   snippets are compile-checked in a scratch build.
3. **generate** emits the Rust facade, builds it and runs interoptopus (producing
   `Interop.cs` and the names sidecar), then emits the public C# layer and smoke tests as a
   deterministic set of files.

## Service contract

The lifecycle mirrors CppMcp's interop tools (`configure_interop`, `analyze_interop`,
`get_unresolved_interop_methods`, `resolve_interop_methods`, `generate_interop`).

- `ConfigureAsync(manifestPath, configuration)`: any change invalidates readiness.
- `AnalyzeAsync(manifestPath)`: writes no outputs; returns total, automatic, unresolved and
  excluded counts and whether generation is ready.
- `GetUnresolvedAsync(manifestPath)`: returns the same next batch until the manifest changes.
- `ResolveAsync(manifestPath, decisions)`: each decision is checked against the item's
  signature hash.
- `GenerateAsync(manifestPath, worktreePath)`: refuses a manifest that is not ready or is
  stale; writes atomically, and only into the given transaction worktree.

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
  `<Root>.Remote.Fetch.Outcome`.

interoptopus itself needs no change for this.

## Hand-written runtime (per target)

- A persistent repository holder (a mutex-guarded repository or a pool).
- The error mapper (the planned error contract, P0b in the GixSharp handoff).
- Conversion helpers.
- Later, the cursor template.

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

## Open decisions

- Root namespace prefix for the public layer (for example `Gix` or `GixSharp.Generated`).
- Object ID representation (hex or binary).
- Filesystem path representation.
- Whether `gix-ffi` moves into GixSharp with its git history.

## Todo

### Prerequisites (gitoxide fork, first)

- [x] R1 Merge or drop the three unmerged slices (merge/snapshot, sparse-checkout, read-tree recovery). Integrated on gitoxide `main`: sparse-checkout `19d524c8`, read-tree recovery `219b4788`, merge/snapshot `3d5b6f94` (one shared read-tree publication and recovery path); 13 superseded transactions rolled back.
- [ ] R2 Move `gix-ffi` to the next interoptopus release.
- [ ] R3 Make every export return `ffi::Result` (no plain-value returns).

### Engine (this repository)

- [x] E1 Pick the `rustdoc-types` release whose `FORMAT_VERSION` is 60; create the engine crate with `extract`, `analyze` and `generate` subcommands and JSON input/output. Integrated at `97c9ffbc`: `rustdoc-types` 0.60.0; `analyze` on `gix.json` reports 682 methods and 22 free functions (704).
- [ ] E2 `extract`: crate list from `Cargo.lock`, plus caching.
- [ ] E3 Model, classifier and signature hashes; `analyze` writes the manifest. Check the result against the 704-function reference count.
- [ ] E4 Decision validation and the compile check for snippets.
- [ ] E5 Rust facade emitter (automatic, handle, convert and record kinds) plus inventory registration. Facade types get unique module-qualified names (see Naming and namespaces).
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