# Immutable C# plugin staging for parallel tests

Status: **implemented.** Landed in `d86c0c72cc18c8faf3418d8be08940b5e646f11f`. The
deterministic Windows regression and full workspace validation passed.

This is a Windows test-infrastructure correction. The generated plugin API and runtime loading
contract do not change.

## Problem

The C# backend integration tests generate and build plugin projects in shared repository
directories. `cargo-nextest` runs individual tests in separate processes, and several tests can
prepare or load the same logical plugin, including `functions_behavior.dll`.

`PluginBuildLock` currently serializes generation, `dotnet build`, and copying into
`_plugins/`. The lock is released when `prepare_plugin` returns. The .NET runtime then loads the
staged DLL and can keep that file mapped for the rest of the test process. A later process can
legitimately rebuild the logical plugin and attempt to overwrite the same staged pathname while
the earlier process still maps it. Windows rejects that overwrite with a sharing or access-denied
error.

The retained failure was `PermissionDenied` at the outer `prepare_plugin` call after
`dotnet build` succeeded. The existing error propagation did not identify the exact filesystem
operation. The test passed unchanged on retry, so the historical failure is evidence of a
state-dependent race, not proof of one particular syscall.

The timestamp-based `stage_is_current` check avoids many unnecessary writes, but timestamps are
not a synchronization primitive and cannot make an overwrite of a mapped pathname safe.

## Invariant

Once a staged DLL pathname is published to a loader, that pathname is immutable. No later build,
test process, or retry overwrites it.

A changed built DLL receives a different directory. The DLL filename itself is preserved because
the dynamic loader derives the managed assembly name from that filename.

## Selected design

Generation and `dotnet build` remain serialized by the existing per-logical-plugin lock at:

```text
_plugins/.lock-<name>
```

After a successful build, the harness reads the built DLL bytes and computes a deterministic
fingerprint. It stages the file at:

```text
_plugins/by-content/<fingerprint>/<name>
```

For example:

```text
_plugins/by-content/0123456789abcdef/functions_behavior.dll
```

The staging rules are:

1. If the content path does not exist, create its directory and publish the built bytes there.
2. If it already exists with identical bytes, reuse it without writing.
3. If it exists with different bytes, report a fingerprint collision and do not overwrite it.
4. Return the immutable content path to every loader.

The fingerprint is an address, not a security boundary. Equality is verified before an existing
path is reused, so a collision becomes a loud test-infrastructure error rather than corruption or
an overwrite.

The in-process cache changes from a set of logical plugin paths to a map from logical plugin key to
the immutable staged path. This preserves the current build-once-per-process behavior and lets all
threads in `concurrent_same_plugins_work` reuse the same published path.

Nested staged DLLs remain ignored by Git. Old content versions are deliberately not deleted during
a test run because another process may still have them loaded. They are small generated test
artifacts and disappear with a transaction worktree; a separate bounded cleanup policy can be
added later if accumulation in long-lived checkouts becomes material.

## Diagnostics

Every fallible filesystem boundary in plugin preparation records its operation and paths:

- creating the staging or project directory;
- acquiring the per-plugin lock;
- writing generated sources;
- launching `dotnet build`;
- reading or fingerprinting the built DLL;
- creating the content directory;
- reading an existing staged DLL;
- publishing a new immutable DLL.

The outer `dll_path_for` panic remains the test failure boundary, but its source message will now
identify the failed operation.

## Deterministic regression

A helper-level test stages byte sequence A, opens the resulting path without write sharing on
Windows, changes the built file to byte sequence B, and stages again while the first handle remains
open.

The test establishes that:

- a direct overwrite of the first path is rejected on Windows, proving the fixture exercises the
  historical constraint;
- staging B succeeds at a different path;
- the first path still contains A;
- staging B again reuses the same second path without rewriting it.

The path and content assertions run on every platform; the no-write-share assertion is
Windows-specific.

## Alternatives rejected

Nextest test-group serialization would only coordinate processes within one nextest invocation. It
would not protect overlapping local and guarded runs or another tool invoking the same tests.

Holding the existing marker lock for the lifetime of a loaded assembly would require propagating a
lease through every loader and handling abrupt process termination. It also retains the shared
mutable pathname.

Changing the runtime to load assemblies from bytes or introducing collectible load contexts is
outside the test-staging boundary.

## Verification

Required focused checks:

- the deterministic immutable-staging regression;
- `backend_plugins::concurrent::concurrent_same_plugins_work`;
- the defining, synchronous-loading, and asynchronous-loading tests for
  `functions_behavior.dll`.

Required completion checks:

- the complete `interoptopus_csharp` target, including generated C# execution;
- workspace `cargo-nextest` validation from the exact candidate;
- no tracked plugin DLL or generated cache artifact enters the commit.

A green run establishes that the harness no longer needs to overwrite a previously published DLL
pathname. It does not establish that Windows will never report an unrelated filesystem denial, so
the added operation-specific context remains part of the deliverable.
