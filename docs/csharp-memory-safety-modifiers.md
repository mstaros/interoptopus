---
chat_url: 'https://chatgpt.com/c/6a93ddf0-1a9c-83ed-964b-cd8e13a5ff01'
---

# C# `safe` and `unsafe` generation for updated memory-safety rules

Status: **proposed.**

This document defines how the C# backend preserves Rust caller-safety contracts and emits the
C# `safe` and `unsafe` modifiers introduced by the .NET 11 / C# preview updated
memory-safety rules. It also separates source API safety from the unsafe mechanics of generated
FFI transport code.

The design does not add a Roslyn package dependency. Interoptopus generates source; the
consumer's .NET SDK compiler validates it.

## Objective

When updated memory-safety generation is selected:

- every generated `[LibraryImport]` declaration has an explicit `safe` or `unsafe`
  classification;
- a Rust `unsafe fn` remains caller-unsafe in direct C# declarations and generated overloads;
- generated managed wrappers remain safe only when they establish all required invariants;
- raw service, string, vector, and wire-buffer entry points are not declared safe merely because
  their Rust transport implementation is written as a safe function;
- unknown safety information produces a diagnostic instead of silently becoming safe; and
- unsafe operations inside generated bodies use scoped `unsafe { ... }` blocks.

The native ABI, symbol names, function IDs, and calling conventions do not change.

## Current information loss

The proc-macro frontend already observes function unsafety:

- `crates/proc_macros_impl/src/function/model.rs` stores
  `input.sig.unsafety.is_some()` in `FunctionModel.is_unsafe`.
- `crates/proc_macros_impl/src/function/emit.rs` uses that value when it re-emits the Rust
  function.

The value is then discarded:

- `crates/core/src/lang/function.rs::Function` has no safety field.
- `crates/backend_csharp/src/lang/functions/mod.rs::Function` has no safety field.
- the original-function and overload model passes therefore cannot propagate safety;
- `templates/rust/fns/rust.cs` and
  `templates/rust/fns/overload/simple.cs` cannot choose a modifier.

The checked-in generated reference output currently contains 291 `[LibraryImport]`
declarations and none has a `safe` or `unsafe` modifier.

The reference C# projects already use `LangVersion=preview` and target `net11.0`, but they do
not currently opt into the updated memory-safety rules. A successful reference build therefore
does not prove compatibility with those rules.

## Safety concepts

Three concepts must remain distinct.

| Concept | Meaning | Consumer |
|---|---|---|
| Function call safety | Whether calling an inventory function requires caller participation in an unsafe audit | Raw C# declaration and free-function wrappers |
| Service member safety | Whether the original Rust constructor or method was declared unsafe | High-level generated C# service API |
| Unsafe lexical context | Permission for pointer dereferences, function-pointer calls, and other unsafe operations inside a C# body | Scoped `unsafe { ... }` block |

The C# `unsafe` member modifier under the updated rules expresses the first or second concept.
It no longer establishes the third. A generated caller-unsafe method can consequently require
both an `unsafe` modifier and an `unsafe { ... }` block.

## Core function safety

Add a safety value to the existing core `Function` model:

```rust
pub enum FunctionSafety {
    Safe,
    Unsafe,
    Unknown,
}
```

For an ordinary `#[ffi]` function:

- a Rust `fn` produces `FunctionSafety::Safe`;
- a Rust `unsafe fn` produces `FunctionSafety::Unsafe`.

`Unknown` represents inventories created by an older serializer, manual construction without
a classification, or another producer that cannot prove the contract. It must not be treated as
`Safe`.

Safety is not part of `FunctionId`. The ID describes native identity and ABI shape; changing a
caller's audit obligation does not create a different native function.

When the core model is deserialized, a missing function-safety field defaults to `Unknown`.
New proc-macro output always supplies an explicit value.

## Service member safety

Keep the existing service topology unchanged:

```rust
pub struct Service {
    pub ty: TypeId,
    pub ctors: Vec<FunctionId>,
    pub destructor: FunctionId,
    pub methods: Vec<FunctionId>,
    pub member_safety: BTreeMap<FunctionId, FunctionSafety>,
}
```

The constructor and method vectors remain identity and routing collections. They are heavily
used for inventory lookup, visibility filtering, overload discovery, interface generation, and
output ordering. Replacing their elements with pairs would force safety metadata through passes
that only need IDs.

`member_safety` records the original Rust API contract:

- every ID in `ctors` and `methods` has one entry in newly generated metadata;
- no other IDs are accepted;
- a missing entry means `Unknown`;
- the synthesized destructor has no entry.

The destructor is governed by generated ownership policy: its raw entry point consumes a service
handle and is unsafe, while the managed `Dispose()` wrapper remains safe after validating and
owning that handle.

This map is necessary because a service has two different safety surfaces. A safe Rust method
may have an unsafe generated transport function that dereferences a raw instance pointer. The raw
C# import is unsafe, but the high-level C# instance method can be safe. Conversely, an originally
unsafe Rust service method must remain unsafe in the high-level API.

The proc-macro service model must capture `sig.unsafety` for both constructors and methods
before generating their transport functions. The generated `ServiceInfo::service()` metadata
then populates the complete map.

For serde compatibility, a missing `member_safety` field deserializes as an empty map, whose
lookups are `Unknown`. It does not mean that all members are safe.

## C# model propagation

The C# function model gains a safety value. Original free functions copy it from the core
function. Every generated overload copies the safety of its source function; overload shape does
not discharge arbitrary preconditions documented by an unsafe Rust function.

The C# service model keeps its existing `ctors`, `methods`, `destructor`, and immutable
`sources` ID collections. It translates the core service's member-safety lookup once. Service
output uses that lookup for high-level constructors and methods.

This avoids changing the existing ID-only chains and the overload pass that replaces
`service.methods` with an expanded renderable list.

## Effective raw-import safety

The backend may strengthen a source classification to `Unsafe`; it must never weaken one.

The effective safety of a raw import is:

1. `Unsafe` when the core function is `Unsafe`;
2. `Unsafe` when a known generated transport role requires raw pointer or ownership invariants;
3. `Safe` when the core function is `Safe` and no transport rule strengthens it; or
4. an error when the result remains `Unknown`.

Known transport roles include service instance/destructor entry points and raw helpers for
UTF-8 strings, vectors, and wire buffers. These roles must be obtained from existing service and
pattern metadata, not from function-name matching.

A high-level wrapper can hide an unsafe raw import only when its generated code establishes the
required invariant. In that case it has no caller-safety modifier and contains a scoped unsafe
call.

## C# emission

Direct and simple-overload `[LibraryImport]` declarations emit an explicit modifier:

```csharp
[LibraryImport(NativeLib, EntryPoint = "safe_function")]
public static safe partial int safe_function(int x);

[LibraryImport(NativeLib, EntryPoint = "unsafe_function")]
public static unsafe partial nint unsafe_function(nint x);
```

The same classification applies to all partial declarations of the same generated member.

For ordinary managed wrappers:

- do not emit `safe`; it is unnecessary noise outside declarations requiring an explicit
  classification;
- emit `unsafe` when the original API contract requires the caller to enter an unsafe context;
- use a scoped `unsafe { ... }` block for unsafe operations in the body, independently of the
  member modifier.

An unsafe source function remains unsafe through ref, marshalling, body, and async overloads.
The generator cannot prove that changing parameter shape discharges arbitrary source-level
preconditions.

## Updated-rules template audit

Adding modifiers to `[LibraryImport]` declarations is not sufficient for an updated-rules
build. Existing generated code also uses the old meaning of `unsafe`.

The implementation must audit at least:

- unmanaged composite and enum-union templates;
- UTF-8 string, vector, and wire-buffer wrappers;
- service constructors, methods, destruction, and async bodies;
- reverse-interop trampolines and function-pointer calls;
- task handles and custom marshallers.

Generated `unsafe` type declarations must be removed because the updated rules reject
`unsafe` on types. Unsafe operations move into scoped blocks.

Fields in explicit-layout generated unions receive the explicit field classification required by
the updated rules.

Existing methods marked `unsafe` only to create a lexical context must be reconsidered. If the
wrapper establishes all invariants, remove the caller-unsafe modifier and retain only a scoped
unsafe block. If the caller must participate in the audit, retain the modifier and also add the
scoped block.

## Identifier compatibility

`crates/proc_macros_impl/src/forbidden.rs` already rejects `unsafe` as a generated-language
identifier but does not reject the new contextual keyword `safe`.

The initial implementation adds `safe` to the same forbidden-name policy and adds a UI test.
Escaping C# identifiers instead would be a broader naming-policy change and is outside this
design.

## Compatibility

This design changes generated C# source but not the native ABI.

Adding fields to the public core `Function` and `Service` structures affects external Rust
code that constructs them with struct literals. Existing serialized inventories remain readable
through serde defaults, but their safety is `Unknown` and updated-rules generation will require
regeneration or an explicit migration.

The existing `Service::new` call shape may remain source-compatible by initializing an empty
safety map. Proc-macro-generated services must then populate the map explicitly. Manual services
created through the old call remain valid inventory values but cannot be emitted under strict
updated rules until classified.

No automatic compiler-version detection is used. Generated output must be deterministic and
must not depend on which SDK happens to run Interoptopus.

The activation surface is intentionally not invented here. This fork can make updated-rules
generation unconditional because its reference output targets `net11.0` preview. If the C#
backend must continue producing source for older compilers, the configuration shape must be
agreed before implementation.

## Validation

The change requires tests at each information boundary.

1. Proc-macro/core tests prove that safe and unsafe free functions produce the corresponding
   core safety value.
2. Service tests prove that safe and unsafe constructors and methods populate
   `member_safety`, while transport-function safety remains independent.
3. Serialization tests prove that missing fields become `Unknown`, never `Safe`.
4. C# snapshots contain one safe and one unsafe free function, including a simple overload.
5. Service snapshots prove that a safe high-level method can call an unsafe raw transport import
   inside a scoped block, and that an unsafe source method remains caller-unsafe.
6. Built-in helper snapshots prove that raw string, vector, and wire imports are unsafe while
   their invariant-establishing managed wrappers remain safe.
7. A UI test rejects `safe` as an incompatible exported identifier.
8. A dedicated `net11.0` Preview 7 fixture opts into the updated memory-safety rules and
   compiles the complete generated output.
9. Compile-fail call-site tests prove that an unsafe generated member cannot be called outside an
   unsafe context, while a safe member can.

The current reference build remains useful, but it is not the acceptance gate until the updated
rules are explicitly enabled.

## Implementation sequence

The work should land as independently reviewable commits:

1. core function safety, service-member map, proc-macro propagation, serde behavior, and Rust
   tests;
2. C# model propagation, effective raw-import classification, import templates, and focused
   snapshots;
3. service and built-in wrapper propagation;
4. updated-rules unsafe-context migration and the opt-in .NET 11 compile gate.

Each commit must leave its affected workspace tests passing. The final commit must compile the
full generated reference project under the updated memory-safety rules.

## Non-goals

This design does not:

- infer arbitrary user-function safety from the presence of pointer parameters;
- make an unsafe Rust contract safe because a C# overload has a friendlier parameter type;
- change native signatures, IDs, symbol names, or calling conventions;
- add a dependency on `Microsoft.CodeAnalysis.CSharp`; or
- define a general cross-language keyword-escaping policy.
