# Rust allocation observability for C# ownership tests

Status: approved for implementation.

This is test infrastructure for the C# reference suite. It closes one specific verification gap left
by the call-scoped marshalling change: the suite can observe that a borrowed managed owner remains
usable, but it cannot currently observe whether the call changed live Rust allocation ownership.

## Objective

Under an opt-in `reference_project` feature, count live allocations and live bytes made through
Rust's global allocator and expose both gauges through two `#[ffi]` functions. Generated C# tests
use those gauges to verify that repeated shared-borrow calls leave the live Rust allocation snapshot
unchanged and that disposing the managed owner releases the allocations it owns.

The measured invariant is **no net Rust allocator ownership change across a borrow**. It is not the
stronger assertion that no allocation or deallocation event occurred during the call.

## Scope

The first measured cases are deliberately tied to the completed borrow-marshalling change:

- a directly owned `Utf8String` passed to `pattern_string_11(in s)`;
- a `UseString` composite, containing two owned `Utf8String` values, passed to
  `pattern_string_6a(in value)`.

The existing by-value, mutable write-back, option/result guard, and usability tests remain the
behavioral controls.

The following are outside this transaction:

- `ffi::Vec<T>` and wire-buffer lifecycle coverage;
- callback closure, async task-handle, and service `Box`/`Arc` lifecycle coverage;
- C# `GCHandle` and `Marshal.AllocHGlobal` accounting;
- allocations made by native libraries or allocators other than Rust's selected global allocator;
- pointer-identity tracking, use-after-free detection, and direct double-free diagnosis.

Those are separate ownership surfaces. The counters introduced here may support later tests for
some of them, but this transaction does not claim that coverage.

## Build and inventory wiring

`reference_project` gains a disabled-by-default `allocation-tracking` feature. The
`interoptopus_csharp` test target enables that feature on its `reference_project`
dev-dependency. This is required in two places at once:

1. `reference_project::inventory()`, linked into the Rust test binary, must contain the probe
   functions so the generated C# bindings declare them.
2. The dev-dependency cdylib copied by `stage_reference_cdylib()` must contain the same exported
   functions when the C# suite loads it.

No nested `cargo build` is added. The existing staging path continues to copy the already-built
dev-dependency cdylib from the Rust test binary's `deps` directory.

Without the feature, the allocator wrapper, probe functions, and inventory entries do not exist.
The reference project remains unchanged for its normal builds.

## Allocator accounting

A feature-gated allocator wraps `std::alloc::System`. It contains no heap-backed state and updates
only relaxed atomics:

- successful `alloc` and `alloc_zeroed` increment live allocation count and add
  `Layout::size()` to live bytes;
- `dealloc` decrements the count and subtracts the layout size after delegating to
  `System`;
- successful `realloc` keeps the allocation count unchanged and adjusts live bytes by the
  difference between the old and new sizes;
- failed allocation or reallocation leaves the gauges unchanged.

The FFI probes return `u64`:

- `__test_live_bytes()`;
- `__test_live_allocations()`.

They are gauges, not cumulative event counters. Equal before/after values establish equal live
state; they cannot exclude a balanced allocation/deallocation pair inside the interval.

## Deterministic C# assertions

The allocation-sensitive suite cannot share the native library with parallel tests while taking a
global snapshot. The C# test assembly therefore disables test parallelization. This is a correctness
condition for the gauges, not a performance choice.

Each measured test:

1. calls the probes before constructing its owner, which also completes native-library loading;
2. constructs the managed owner and confirms that both live gauges increased;
3. records the owned snapshot;
4. performs repeated `in` calls and requires both gauges to equal the owned snapshot;
5. disposes the owner and requires both gauges to return to the baseline.

The owner is held in a scoped `using` declaration so an assertion failure still releases it and
does not contaminate later allocation tests.

The million-call heuristic in `string_by_in_does_not_observably_consume` is replaced by a bounded
repeat count plus direct measurement. Repetition still exercises the marshaller lifecycle, while
the gauges provide the assertion the old TODO lacked.

## Verification

Required generated-text checks:

- both probe functions appear in the generated C# interop surface;
- existing shared-borrow declarations still use `in` and their in-marshallers still call
  `AsUnmanaged()`.

Required execution checks:

- direct `Utf8String` ownership increases both live gauges, repeated borrowing preserves them,
  and disposal restores the baseline;
- composite `UseString` ownership does the same;
- the existing by-value and mutable-reference behavior remains green.

Validation runs the complete `interoptopus_csharp` test target, including generation snapshots and
the generated C# executable suite. The transaction must report both the Rust test step and the C#
execution step before integration.

## What green verification establishes

For the two exercised ownership shapes, while the test assembly is serialized:

- constructing the owner creates live Rust allocator state;
- the tested shared-borrow path produces no net change in that state;
- disposing the owner restores the measured baseline.

## What it does not establish

A green run does not prove that no allocator event occurred during a borrow, that every native
ownership family is covered, or that leaks and double frees are impossible elsewhere. A double free
will normally terminate the process before an assertion can diagnose it. The counters also cannot
see C#-side unmanaged allocations or managed handles. These limits remain part of the verification
record.
