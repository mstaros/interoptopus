# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### ⚠️ Breaking

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
  through a case constructor or factory." Construct through the generated case constructors or
  factories instead. Class-backed enums are unaffected, since their empty state is a null
  reference rather than a cleared `_hasValue`.

- **`IsX` and `AsX` on a default struct-backed enum no longer report variant zero.** `IsOk` read
  `_variant == 0`, and `AsOk()` tested only the variant; neither consulted `_hasValue`. A
  default-constructed value therefore reported `HasValue == false` and `IsOk == true` at the same
  time, and `AsOk()` returned a zeroed payload read out of uninitialised memory instead of
  throwing. Both now test `_hasValue` first: `IsX` is `false` on an empty value, and `AsX` throws
  `ExceptionForVariant()` exactly as it already did on a variant mismatch. Code that called `AsX`
  on a default value and used the result was consuming a fabricated value and now receives an
  exception instead. Class-backed enums are unaffected — they carry no `_hasValue`, and their
  empty state is a null reference.

- **Class-backed enums no longer expose a public parameterless constructor.** `new EnumX()` on a
  class-backed enum previously produced a variant-zero instance from outside the type — a variant
  the Rust side never sent, and one that `default(EnumX)` (a null reference) does not otherwise
  admit. The constructor is now `private`; construct through the generated factories instead.
  Struct-backed enums are unaffected, since their empty state is carried by `_hasValue`.

## [0.16.4](https://github.com/ralfbiedert/interoptopus/compare/interoptopus_csharp-v0.16.3...interoptopus_csharp-v0.16.4)

### 🐛 Bug Fixes


- Prevent async task handle use-after-free - ([b56f758](https://github.com/ralfbiedert/interoptopus/commit/b56f7587acd7fecca4bf69f573cc29d820e9450d))

