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

## Rust-owned queries

Return an `ffi::Iterator<T>` from a registered Rust function or service method:

```rust
use interoptopus::ffi;

#[ffi]
pub fn numbers() -> ffi::Iterator<u32> {
    ffi::Iterator::new(vec![1, 2, 3, 4, 5].into_iter())
}
```

Register the export with `function!(numbers)` as usual. The Rust library backend
automatically emits `IteratorUint` and its extension class in the namespace selected
by the output dispatch. Import that namespace in C#:

```csharp
using var query = Interop.numbers()
    .Where(x => x % 2 == 0)
    .Take(2);
bool found = query.Any(x => x > 3);
```

`Where` accepts `Func<T, bool>`. `Where` and `Take` are deferred; `Any`
evaluates in Rust, short-circuits, and releases the pipeline. Negative `Take`
counts behave like zero. Managed predicates are retained until the query is
released. A predicate exception stops traversal and is rethrown by `Any` after
cleanup. Each element reaching a managed predicate requires a native-to-managed
callback; this does not compile C# lambdas into Rust code.

Queries are owning, single-pass objects. Every operator consumes its receiver;
use its returned stage and dispose unfinished queries with `using`. Earlier
wrappers cannot be reused, although disposing them is harmless. This also applies
when passing a query by value back into Rust. Unlike `IEnumerable<T>`, a consumed
query cannot be enumerated again. Explicit disposal is required for abandoned
queries whose predicate captures its own query wrapper.

The Rust source must be `Send + 'static`. A traversal over a shared collection
can retain an `Arc` to that collection and produce owned elements; a traversal
borrowing a local collection cannot be exported. The source remains in Rust and
is not materialized by the binding layer.

This first version supports `Where`, `Take`, `Any()`, and `Any(predicate)`
for scalars, plain enums, and structs composed of those values. Pointer, slice,
string, array, payload-enum, and owning-wrapper elements are rejected during
generation. Element structs are copied through their generated unmanaged
representation before invoking the predicate.

The initial operators use `std::iter`. Additional operators can use `itertools`
as an ordinary dependency; no itertools fork is required.
