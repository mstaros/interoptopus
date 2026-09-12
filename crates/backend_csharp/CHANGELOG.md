# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### ⚠️ Breaking

- Generated service wrappers now own native resources through a private, service-specific
  `SafeHandle`, including cleanup when `Dispose()` is omitted. Native destruction must be safe
  on the finalizer thread or the thread releasing the last call, and must not panic or throw;
  generation warns about this requirement without claiming to verify it. `ReleaseHandle`
  contains managed interop exceptions; cleanup failures no longer propagate from `Dispose()`.
  Call scopes retain the handle through native completion or cancellation acknowledgement.
  Interlocked borrow guards and immediate rejection of calls after disposal are preserved.
  Arbitrary raw pointers and returned borrows do not gain ownership or lifetime guarantees.

- Generated C# service wrappers use Interlocked borrow guards when exported signatures
  contain mutable service access. Overlapping mutable/mutable or mutable/shared calls
  now throw `InvalidOperationException` before native entry, including callback reentry,
  incompatible service aliases, and direct span methods. Shared borrows may coexist;
  asynchronous scopes retain their borrow through completion or cancellation acknowledgement.
  Generation warns with the affected service and exports. The guards cover managed call
  duration only: raw pointers, returned borrows, and Rust Send/Sync or thread-affinity
  requirements still need caller coordination.

- **Read-only custom-marshalled pointer overloads now use `in T` instead of `ref T`.**
  Their dedicated `ManagedToUnmanagedIn` marshaller borrows through `AsUnmanaged()` and does
  not move ownership out of the managed value or write it back after the call. Call sites that
  explicitly passed `ref` must use `in` (or the permitted unadorned call form). Raw `IntPtr`
  declarations, read/write pointers, and direct / `AsIs` pointer overloads remain unchanged.

- **Unit-only enums are now emitted as plain C# enums.** An enum whose variants all carry no
  payload was previously generated as a struct with `IsX`/`AsX` accessors, per-variant case
  types, `Value`, `HasValue`, `TryGetValue` and a custom marshaller. It is now a plain C# enum
  whose underlying type is the Rust discriminant — `public enum EnumDocumented : byte`. None of
  those members exist on a plain enum, so any code referencing them fails to compile. Replace
  `x.IsFail` with `x == Error.Fail`, and construct values as ordinary enum members. Enums with
  at least one payload-carrying variant are unaffected and keep the union projection.

- **Marshalling out a default struct-backed enum now throws.** A default-constructed value has
  no variant set, and previously marshalled to Rust as variant zero — a variant the Rust side
  never sends. `ToUnmanaged`/`AsUnmanaged` now throw `InvalidOperationException` with
  "Cannot marshal a default X: it is empty and corresponds to no Rust variant. Construct one
  through a case constructor or factory." `Wire<T>` serialization delegates to the same
  classifier instead of emitting a separate "Unknown variant" error. Construct through the
  generated case constructors or factories instead. Class-backed enums have no `_hasValue`;
  their null state is covered
  separately below.

- **Null class-backed unions now fail with `InvalidOperationException` before native entry.**
  Direct parameters (including `ManagedToUnmanagedIn` borrows), composite fields, nested union
  payloads and non-blittable slice elements use one contract: the message names the failing
  type, member or index and says the value corresponds to no Rust variant. Previously these paths
  reached an instance call on null and threw `NullReferenceException`; slice null rejection now
  also occurs before `Marshal.AllocHGlobal`.

- **`IsX` and `AsX` on a default struct-backed enum no longer report variant zero.** `IsOk` read
  `_variant == 0`, and `AsOk()` tested only the variant; neither consulted `_hasValue`. A
  default-constructed value therefore reported `HasValue == false` and `IsOk == true` at the same
  time, and `AsOk()` returned a zeroed payload read out of uninitialised memory instead of
  throwing. Both now test `_hasValue` first: `IsX` is `false` on an empty value, and `AsX` gets
  `InvalidOperationException` from `ExceptionForVariant()` before variant zero can be read as a
  mismatch. Calls on a well-formed but different variant still receive `EnumException`. Code that
  caught `EnumException` for a default value must catch `InvalidOperationException` instead.
  Class-backed enums carry no `_hasValue`; their empty state is the null contract above.

- **Class-backed enums no longer expose a public parameterless constructor.** `new EnumX()` on a
  class-backed enum previously produced a variant-zero instance from outside the type — a variant
  the Rust side never sent, and one that `default(EnumX)` (a null reference) does not otherwise
  admit. The constructor is now `private`; construct through the generated factories instead.
  Struct-backed enums are unaffected, since their empty state is carried by `_hasValue`.

### 🐛 Bug Fixes

- Generated managed service calls now retain native ownership while a call is active,
  including service arguments and direct span methods. Async overloads retain it through
  completion or cancellation acknowledgement. `Dispose()` rejects new calls immediately
  and destroys the native service exactly once after acquired calls release it, without
  waiting inside callbacks. Raw `IntPtr` imports still require caller-managed lifetimes;
  lifetime retention is separate from the mutable-access guards described above.

- **Generated custom-marshalled types now work in consumer-owned `LibraryImport` declarations.**
  Their nested `Unmanaged`, default `Marshaller`, and read-only `InMarshallerMeta` /
  `InMarshaller` helpers are public so the .NET source generator can reference them across
  assembly boundaries. The private `MarshallerMeta` carrier and internal `WireBuffer`
  implementation remain hidden.

- **`ToString()` on a default struct-backed union now returns `"<empty>"`.**
  The all-zero managed value has no Rust variant; it previously fell through the discriminant-only
  formatter and appeared as variant zero. The generated guard now checks `_hasValue` before variant
  matching. Valid variants, class-backed unions and plain enums retain their existing formatting.

- **Pointer-sized discriminants in manual enum inventories now retain native width.**
  `Layout::Primitive(Isize)` and `Usize` map to C# `nint` and `nuint` instead of silently
  falling back to `int`. Unit-only enums with those discriminants remain struct-backed with an
  unmanaged mirror and custom marshaller, because C# does not permit native integers as enum
  underlying types. Ordinary `#[ffi]` enums are unchanged: the macro selects a fixed-width repr.

## [0.16.4](https://github.com/ralfbiedert/interoptopus/compare/interoptopus_csharp-v0.16.3...interoptopus_csharp-v0.16.4)

### 🐛 Bug Fixes


- Prevent async task handle use-after-free - ([b56f758](https://github.com/ralfbiedert/interoptopus/commit/b56f7587acd7fecca4bf69f573cc29d820e9450d))

