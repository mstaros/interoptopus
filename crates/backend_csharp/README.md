# `interoptopus_csharp`

C# backend for [Interoptopus](https://crates.io/crates/interoptopus).

Generates idiomatic C# bindings from a Rust FFI library, including classes for services, delegates for callbacks,
and most Interoptopus patterns.

## Usage

Add the crate as a dependency:

```toml
[dependencies]
interoptopus_csharp = "..."
```

Then write a test that builds an inventory and runs the backend:

```rust
use interoptopus::inventory::RustInventory;
use interoptopus::{ffi, function};
use interoptopus_csharp::RustLibrary;

#[ffi]
pub fn my_function(x: u32) -> u32 { x + 1 }

fn generate_bindings() -> Result<(), Box<dyn std::error::Error>> {
    let inventory = RustInventory::new()
        .register(function!(my_function))
        .validate();

    RustLibrary::builder(inventory)
        .dll_name("my_lib")
        .build()
        .process()?
        .write_buffers_to("bindings/")?;

    Ok(())
}
```

This produces an `Interop.cs` file in `bindings/` with `[LibraryImport("my_lib")]` declarations
and idiomatic C# wrappers for all registered items.

For multi-file output or custom namespaces, use a [`Dispatch`](https://docs.rs/interoptopus_csharp/latest/interoptopus_csharp/dispatch/struct.Dispatch.html):

```rust
use interoptopus_csharp::dispatch::Dispatch;
use interoptopus_csharp::lang::meta::FileEmission;
use interoptopus_csharp::output::Target;

let dispatch = Dispatch::custom(|item, _| match item.emission {
    FileEmission::Common => Target::new("Interop.Common.cs", "My.Company.Common"),
    FileEmission::Default => Target::new("Interop.cs", "My.Company"),
    FileEmission::CustomModule(_) => Target::new("Interop.cs", "My.Company"),
});
```

## Callbacks

Named Rust callbacks declared with `callback!` generate `System.Func<...>` parameters
when they return a value, and `System.Action<...>` parameters when they return `()`.
This is the default for callback constructors, exported function overloads, and service
methods. Both inline lambdas and existing `Func`/`Action` variables can be passed directly.

For example, `callback!(Transform(value: u32) -> u32)` accepts a C#
`Func<uint, uint>`, while `callback!(Visit(value: u32))` accepts an `Action<uint>`.
Callbacks without arguments use `Func<TResult>` or `Action`. Scalar Rust/FFI
booleans use `bool` in these standard delegate signatures.

Signatures with more than sixteen input parameters, or by-reference parameters or
returns, retain custom named delegates. Bare `extern "C" fn` delegates retain their
native calling-convention declarations. The owning callback wrapper and native
marshalling remain responsible for callback state and disposal.

This changes the generated public API: callers naming an old `NameDelegate` type
should use the corresponding `Func` or `Action`. Inline lambda calls retain their syntax.

## Rust.Linq queries

The Rust library backend emits a shared `Rust.Linq.cs` file alongside the library
bindings. Import `Rust.Linq` and call `ToRust()` on any `IEnumerable<T>`:

```csharp
using System.Collections.Generic;
using System.Linq;
using Rust.Linq;

IEnumerable<int> values = new[] { 1, 2, 3, 4 };
using IRustEnumerable<int> query = values.ToRust()
    .Where(x => x > 1)
    .Take(2);
bool found = query.Any(x => x == 3);
```

`ToRust()` returns `IRustEnumerable<T>`, which extends `IEnumerable<T>` and
`IDisposable`. It does not enumerate or copy the source. The query operators
extend this more specific interface, so `System.Linq` and `Rust.Linq` can be
imported together, including projects with implicit `System.Linq` imports.
Plain `IEnumerable<T>.Where(...)` remains normal LINQ; `ToRust().Where(...)`
selects this API. Queries also work with `foreach`, `ToArray()`, and other
existing consumers of `IEnumerable<T>`.

### Execution and ownership

Generated `ffi::Iterator<T>` wrappers implement `IRustEnumerable<T>` directly.
`ToRust()` preserves that object, even when it has been upcast to
`IEnumerable<T>`. Its `Where`, `Take`, and `Any` operators execute through
Rust function pointers. Predicates are ordinary `Func<T, bool>` callbacks:
Rust invokes the C# lambda; the binding does not compile lambdas into Rust.

Other `IEnumerable<T>` sources use deferred managed LINQ. This includes arbitrary
managed element types such as strings and user classes, without FFI marshalling.
`ToRust()` selects the query API; it does not promise native execution or move a
managed collection into Rust. The managed adapter does not take ownership of its
source, and its `Dispose()` is a no-op. Its repeatability and resource lifetime
follow the original enumerable and the usual enumerator disposal rules.
Existing Rust collection wrappers that only implement `IEnumerable<T>` also use
this fallback; native operator execution requires the iterator binding below.

Native iterator queries remain owning and single-pass. `Where` and `Take`
transfer ownership to the returned query. `Any` consumes and releases it.
`GetEnumerator()` transfers ownership to an enumerator without evaluating any
element; `MoveNext()` advances the Rust traversal. Exhaustion, an exception, or
enumerator disposal releases the native state and retained predicates. A
`foreach` break therefore cleans up correctly. Dispose unfinished queries with
`using`; earlier moved wrappers cannot be reused, though disposing them is harmless.
Explicit disposal is necessary if a predicate captures its own query or enumerator.
`Reset()` is unsupported.

A predicate exception stops native traversal and is rethrown after cleanup.
Negative `Take` counts behave like zero. Managed callbacks are rooted until the
native query is released.

### Exposing a Rust traversal

Return an owning `ffi::Iterator<T>` from a registered Rust function or service
method:

```rust
use interoptopus::ffi;

#[ffi]
pub fn numbers() -> ffi::Iterator<u32> {
    ffi::Iterator::new(vec![1, 2, 3, 4, 5].into_iter())
}
```

Register the export with `function!(numbers)` as usual. The generated
`IteratorUint` wrapper lives in the namespace selected by output dispatch;
query extensions live in `Rust.Linq`:

```csharp
using Rust.Linq;

using var query = Interop.numbers().Where(x => x % 2 == 0).Take(2);
bool found = query.Any(x => x > 3);
```

The Rust source must be `Send + 'static`. A traversal over a shared collection
can retain an `Arc` and produce owned elements. It cannot borrow a local
collection. The binding does not materialize the source collection.

Native elements currently support scalars, plain enums, and structs composed of
those values. Pointer, slice, string, array, payload-enum, and owning-wrapper
elements are rejected during generation. Native elements are copied through
their unmanaged representation for C# predicates and enumeration. These
restrictions do not apply to the managed `ToRust()` fallback.

This version provides `Where`, `Take`, `Any()`, and `Any(predicate)`.
Operators return `IRustEnumerable<T>`; use that type for query variables.
Passing a native query back to an FFI function that takes `IteratorUint`
requires casting the native query to that concrete wrapper type.

`Rust.Linq.cs` contains no library-specific code. Compile one copy per C#
compilation. If several binding assemblies share one query API, put that file in
a common referenced assembly and exclude their duplicate copies. Its provider
hooks are public for generated implementations across assemblies and hidden from
normal editor completion.

The iterator ABI now includes a sixth function pointer for enumeration.
Regenerate bindings and rebuild the native library together; the type identity
and API hash change detects stale bindings. Initial operators use `std::iter`;
later operators can use `itertools` as an ordinary dependency, without a fork.
