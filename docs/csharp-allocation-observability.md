# Rust allocation observability for C# ownership tests

Status: **implemented.** Allocator instrumentation landed in `8ad0f888`; deterministic
ownership-family coverage now exercises the existing reference-project APIs listed below.

This is test infrastructure for the generated C# reference suite. It observes live allocations made
through the Rust reference library's selected global allocator so ownership tests can assert more
than continued usability after a call.

## Objective

Under the opt-in `reference_project/allocation-tracking` feature, count live allocations and live
bytes made through Rust's global allocator and expose both gauges through two `#[ffi]` functions.
Generated C# tests compare snapshots around construction, calls, ownership transfer and disposal.

The measured invariant is **no unexpected net Rust allocator ownership change**. It is not the
stronger assertion that no allocation or deallocation event occurred during an interval.

## Covered ownership families

No new Rust type or FFI entry point is required. The reference corpus already exposes each path:

| Family | Existing path | Measured contract |
|---|---|---|
| `ffi::String` | `pattern_string_11(in s)` | repeated direct borrows preserve the owned snapshot; disposal restores baseline |
| Composite-owned strings | `pattern_string_6a(in value)` | repeated composite borrows preserve both owned strings; disposal restores baseline |
| Mutable string write-back | `pattern_string_6b(ref value)` | after the required first write-back, repetition preserves the replacement snapshot; disposal restores baseline |
| `ffi::Vec<T>` | `pattern_vec_1`, `pattern_vec_2` | construction increases live ownership; disposal and by-value transfer each restore baseline |
| Wire buffer | `wire_return_byte_array`, `wire_accept_byte_array` | a returned or serialized Rust buffer is live while managed owns it; disposal or transfer restores baseline |
| Unique service handle | `ServiceVariousSlices` | the `Box`-backed service and its inner `Vec` remain live across a borrowed method call and are freed by disposal |
| Shared service handles | `ServiceMain`, `ServiceDependent` | each `Arc`-backed service adds live ownership; nested disposal returns first to the parent snapshot and then baseline |

The existing by-value, mutable write-back, option/result guard, and usability tests remain behavioral
controls. Allocation assertions supplement them; they do not replace semantic checks.

## Build and inventory wiring

`reference_project` has a disabled-by-default `allocation-tracking` feature. The
`interoptopus_csharp` test target enables it on its `reference_project` dev-dependency. Feature
unification supplies the probes to both places that must agree:

1. `reference_project::inventory()`, linked into the Rust test binary, contains the probe
   functions so generated C# declares them.
2. The dev-dependency cdylib copied by `stage_reference_cdylib()` exports the same functions when
   the generated C# suite loads it.

No nested `cargo build` is added. Without the feature, the allocator wrapper, probes and inventory
entries do not exist, so ordinary reference-project builds are unchanged.

## Allocator accounting

The feature-gated allocator wraps `std::alloc::System`. It contains no heap-backed state and
updates only relaxed atomics:

- successful `alloc` and `alloc_zeroed` increment live allocation count and add
  `Layout::size()` to live bytes;
- `dealloc` decrements the count and subtracts the layout size after delegating to `System`;
- successful `realloc` keeps allocation count unchanged and adjusts live bytes by the difference
  between old and new sizes;
- failed allocation or reallocation leaves the gauges unchanged.

The FFI probes return `u64`:

- `__test_live_bytes()`;
- `__test_live_allocations()`.

They are gauges, not cumulative event counters. Equal snapshots establish equal live state; they
cannot exclude a balanced allocation/deallocation pair inside the interval.

## Deterministic C# assertions

The probes report process-global native state, so the generated C# assembly disables test
parallelization. Every measured test calls a probe before constructing its owner, which also
completes native-library loading.

Tests use nested snapshots rather than absolute allocation sizes:

1. capture the baseline;
2. construct an owner and require both gauges to increase;
3. exercise the ownership operation and compare with the appropriate live snapshot;
4. release or transfer ownership and require both gauges to return to the preceding snapshot;
5. use `using` or `try/finally` so a failed assertion cannot contaminate later allocation tests.

The mutable write-back path is intentionally warmed once before its steady-state snapshot:
`pattern_string_6b` replaces `"hello"/"world"` with `"s1"/"s2"`, so the first call legitimately
changes live byte size. Subsequent calls must not accumulate ownership.

## Visibility boundary

These counters observe allocations made through the Rust cdylib's global allocator only. They do
**not** observe:

- `Marshal.AllocHGlobal` / `Marshal.FreeHGlobal`;
- managed `GCHandle` allocation and release;
- allocations made by another native library or allocator;
- pointer identity, dereferences, or use-after-free;
- callback closure and asynchronous task-handle lifecycles, which remain separate ownership
  surfaces.

Consequently this mechanism cannot verify that item 4b's null guard executes before
`Marshal.AllocHGlobal`; that ordering is on the .NET side and needs separate .NET-side
observability. It can verify only Rust allocator state on paths that reach the reference cdylib.

A double free may terminate the process, which is a failing test run but not a counter diagnosis. If
the platform allocator does not terminate immediately, the gauges may become nonsensical; neither
outcome makes this a direct double-free detector.

## Verification

Required generated-surface checks:

- both probe functions remain in generated C#;
- existing shared-borrow declarations remain `in`, and their in-marshallers still call
  `AsUnmanaged()`.

Required execution checks:

- direct, composite and mutable-write-back string ownership returns to baseline;
- `ffi::Vec<T>` returns to baseline after both disposal and by-value transfer;
- Wire buffers return to baseline after both disposal and by-value transfer;
- unique `Box`-backed and shared `Arc`-backed services return to nested baselines;
- existing behavior tests remain green.

Validation runs the complete `interoptopus_csharp` target, including generation snapshots and the
generated C# executable suite.

Verification on 2026-08-29 completed both focused and full runs. The generated C# suite
passed 222/222 with no skips. The full Cargo run also passed 17 library tests, 2 basic tests,
1 extension test, 59 reference/integration tests (with 2 intentionally ignored memory tests),
2 template tests, and 5 doctests (with 4 ignored examples).

## What green verification establishes

For the explicitly exercised shapes, while the C# assembly is serialized:

- constructing an owner creates observable Rust allocator state;
- tested borrow and write-back paths produce the documented net state;
- disposal or ownership transfer returns to the expected snapshot.

It does not establish repository-wide absence of leaks, double frees or use-after-free. Green means
the enumerated ownership paths satisfy their measured Rust allocator invariants.
