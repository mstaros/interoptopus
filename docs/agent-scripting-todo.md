# Agent scripting TODO

Goal: build the Rust bridge and generated C# once; agents write ordinary C# scripts against host-provided collections.

One task, in priority order:

- [x] P0 — Add a per-execution `Rust.Linq.ScriptScope` that releases abandoned native queries and enumerators, including failures, and supplies a default cancellation token across awaits.
- [x] P1 — Add reusable query views from an explicit fresh-traversal factory. Preserve native iterators' owning, single-pass contract.
- [x] P1 — Give generated async service methods/constructors a consistent `Async` suffix without changing native export names.
- [x] P1 — Expose service collection results as `IRustEnumerable<T>` while keeping concrete native descriptors in generated interop.
- [x] P1 — Verify native ownership, repeated queries, cancellation, and generated C# compilation; update examples and snapshots.
- [ ] P3 — Add native Rust async-stream bindings if the bridge needs asynchronous item production. Current async queries await managed predicates between synchronous native pulls.

## Contracts

Factories are registered once by the bridge/host, not written in every script. A factory must return a fresh traversal each time; this is a live view unless the bridge explicitly snapshots the collection. Arbitrary `ffi::Iterator<T>` exports cannot be inferred to be repeatable.

The generated enumerator stays internal to script usage. Scripts use `foreach`, LINQ, `await foreach`, and `await`. Async service naming is a managed API change; native ABI export names stay stable.

Scripts are authored and executed elsewhere. That application is outside this task. It can use the generated support: keep a script scope alive through consumption of lazy/async results, provide persistent collection objects, and import System.Linq and Rust.Linq. Persistent collections remain application-owned.

## Validation

The reference bindings and benchmark compile; all 308 C# integration tests pass under the configured patched .NET 11 runtime. The transaction gate additionally validates Rust generation, naming collisions, and the updated snapshot before integration.
