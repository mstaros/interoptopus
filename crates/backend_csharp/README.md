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
Use explicit disposal or a `ScriptScope` if a predicate captures its own query or enumerator.
`Reset()` is unsupported.

A predicate exception stops native traversal and is rethrown after cleanup.
Negative `Take` counts behave like zero. Managed callbacks are rooted until the
native query is released.

### Reusable collections and script execution

The consolidated implementation task is [Agent scripting TODO](../../docs/agent-scripting-todo.md).
Scripts are authored and executed in another application; this fork supplies bindings
and shared support, not a scripting engine.

A native `ffi::Iterator<T>` is one traversal. For a persistent Rust collection,
register a fresh-traversal factory once in the bridge/application:

```csharp
// service.Values() opens a new owning traversal over the Rust collection.
IRustEnumerable<uint> values = RustEnumerable.FromFactory(service.Values);
```

A generated service method returning `ffi::Iterator<T>`, or
`ffi::Result<ffi::Iterator<T>, E>`, exposes `IRustEnumerable<T>` in C#.
The underlying import and marshaller retain the concrete native wrapper.
The reusable view invokes the factory only when evaluation starts. Each
enumeration, `Any`, or async traversal gets independent native state; filters
and limits remain deferred. It observes whatever data the factory exposes on
that call, so repeatability does not imply snapshot isolation. The view owns
no traversal between evaluations, and its `Dispose()` is a no-op.

The application may establish one scope around script execution **and result
consumption**:

```csharp
await using var scope = new ScriptScope(cancellationToken);
// Invoke the script and consume any returned lazy/async results inside this scope.
```

Generated native queries and enumerators automatically join the active scope.
Disposal and ownership transfer remove old wrappers from tracking. Scope exit
releases abandoned queries, even if execution fails. `await using` also waits for
pending native stream requests to acknowledge cancellation and release their
state; synchronous `Dispose()` initiates this cleanup without blocking.
Ordinary `foreach` and
LINQ terminals still release traversals immediately. Nested scopes and awaits
are supported. A scope does not take ownership of persistent service objects
or cancel/wait for detached tasks. Await those tasks before leaving the scope.
A captured context cannot create new native queries after its scope closes.

The script itself can use normal query syntax without interop types or a
`using` for each stage:

```csharp
using System.Linq;
using Rust.Linq;

var first = values.Where(x => x > 1).Take(2).ToArray();
var again = values.Where(x => x > 1).Take(2).ToArray();
```

Do not return a live native traversal beyond its scope. Consume/materialize it
inside the scope, or keep the scope alive until the caller finishes consuming it.
An already-reusable factory view may outlive a scope if its persistent collection
is still valid.

### Async queries

The generated support uses the framework `System.Linq.AsyncEnumerable` APIs
(.NET 10 or later; this fork's reference project targets .NET 11). Import both
`System.Linq` and `Rust.Linq`. An async predicate transitions the query to the
standard `IAsyncEnumerable<T>` interface:

```csharp
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Rust.Linq;

using var source = Interop.numbers().Where(x => x > 1); // This filter runs in Rust.
var query = source
    .Where(async x => { await Task.Yield(); return x % 2 == 0; })
    .Take(5);

await foreach (var value in query)
    Console.WriteLine(value);
```

`Where(Func<T, Task<bool>>)` accepts ordinary async lambdas and existing
Task-returning delegates. It also works on subsequent `IAsyncEnumerable<T>`
stages. For cooperative cancellation, use the two-argument overload:

```csharp
using var source = Interop.numbers();
var query = source.Where(async (x, token) =>
{
    await Task.Delay(1, token);
    return x > 2;
});
bool found = await query.AnyAsync(cancellationToken);
```

The token-aware predicate is `Func<T, CancellationToken, ValueTask<bool>>`,
matching framework async LINQ. A predicate may await an already-generated Rust
service method, such as `await service.ReturnAfterMsAsync(x, 1, token)`. There is no
extra native callback protocol for async predicates: C# awaits the predicate
between native pulls. Only the current value is copied; the collection remains
in Rust. Synchronous stages before the transition keep their native dispatch;
subsequent stages use framework async LINQ.

`IRustEnumerable<T>` supports `await foreach` through a generated
`GetAsyncEnumerator` extension. Use `source.ToAsyncEnumerable()` when an API
requires the actual `IAsyncEnumerable<T>` interface or to call framework
operators such as `Select`, `ToArrayAsync`, and `CountAsync`.
`source.AnyAsync()` supports synchronous, Task-returning, and token-aware
predicates; it returns `ValueTask<bool>`. Call `.AsTask()` if a consumer
specifically needs `Task<bool>`. After the transition, normal framework
`AnyAsync`, `Take`, and other operators compose without duplicate signatures.

Async pipelines are deferred and follow standard async enumeration ownership.
Keep the original native source in a `using` scope until the pipeline has
finished. Starting enumeration transfers its native ownership; exhaustion,
early `await foreach` exit, failure, and cooperative cancellation dispose the
active enumerator. Creating an async query, disposing an enumerator before its
first move, or applying `Take(0)` may never start the source. In those cases,
the original source still owns the native state and its `using` scope releases
it. Async query objects themselves are standard `IAsyncEnumerable<T>`, so use
`var query`, not `using var query`. Managed sources retain their ordinary
repeatability. Native sources remain single-pass.

Generated async service methods and constructors end in `Async`. Names already
ending in `Async` keep their suffix; conflicting generated member names are
reported as a generation error. Update callers of previous names, for example
`ReturnAfterMs` becomes `ReturnAfterMsAsync`. Native export names do not change.

Async service calls and Rust.Linq enumeration use the active
`ScriptScope` cancellation token when the caller supplies no cancellable token.
A supplied cancellable token takes precedence. `default` and
`CancellationToken.None` both mean "use the scope default" inside a scope.
Pre-cancelled service calls fail before starting native work. Generated service
calls made inside an async predicate therefore need no repeated token argument;
arbitrary managed operations such as `Task.Delay` still need their token passed
explicitly. Framework operators added after conversion to `IAsyncEnumerable<T>`
retain their standard token rules; the native source still observes the scope token.

Predicates are awaited sequentially. A predicate that does not accept or
observe cancellation must finish before enumeration can unwind; pass the token
through to the awaited operation when prompt cancellation matters.
Synchronous native traversal is still synchronous and is not interrupted by a
token. These adapters neither move work onto a background thread nor export
Rust `Stream` values. They require no native ABI change.

The new `Where` overloads make a bare `null` or an always-throwing lambda
ambiguous. Give those expressions their intended delegate type, for example
`source.Where((Func<uint, bool>)(_ => throw new Exception()))`.

### Native Rust async streams

Use `ffi::AsyncIterator<T>` when producing the next Rust item can itself await.
It wraps a standard [`futures_core::Stream<Item = T>`](https://docs.rs/futures-core/0.3.31/futures_core/stream/trait.Stream.html)
and the existing `AsyncRuntime`; it does not depend on a particular collection
or require an itertools fork.

For example, a bridge service can wrap its own asynchronous traversal:

```rust
use interoptopus::ffi;
use interoptopus::rt::Tokio;

#[ffi(service)]
pub struct Tree {
    runtime: Tokio,
    // Shared tree state owned by this bridge.
}

#[ffi]
impl Tree {
    pub fn create() -> Self {
        Self { runtime: Tokio::new() }
    }

    pub fn nodes(&self) -> ffi::AsyncIterator<u32> {
        // open_node_stream() is application code returning an owned,
        // Send + 'static Stream<Item = u32> over the tree.
        ffi::AsyncIterator::new(open_node_stream(), self.runtime.clone())
    }
}
```

Register the service as usual. The generated method is
`IAsyncEnumerable<uint> NodesAsync()`. A result-wrapped stream,
`ffi::Result<ffi::AsyncIterator<T>, E>`, also exposes `IAsyncEnumerable<T>`;
opening errors follow the normal generated result-to-exception conversion.
Free functions remain on the configured `Interop` class and return the concrete
owning wrapper in the configured binding namespace.

Scripts use ordinary async enumeration and framework async LINQ:

```csharp
using System.Linq;
using Rust.Linq;

await using var scope = new ScriptScope(cancellationToken);

await foreach (var node in tree.NodesAsync()
    .Where(x => x > 10)
    .Take(5))
{
    Console.WriteLine(node);
}
```

Each `MoveNextAsync()` asks Rust for one item. Rust polls the pinned stream on
the supplied runtime and completes the pending request when an item, end, or
failure is available. The binding neither copies the whole collection nor
prefetches another item. The producer may have its own internal buffering.
C# receives a copy of the current value; the same scalar, enum, and plain-struct
element restrictions as synchronous native iterators apply. Async LINQ operators
run in C#; put native filtering in the Rust stream when needed.

A native stream is owning and single-pass. Its creation is synchronous; waiting
for individual items is asynchronous. This pattern cannot currently be returned
from a Rust `async fn`; return the descriptor from a regular function and put
asynchronous initialization inside the stream. The runtime is retained for the
traversal and its final pending request. Reuse a service's runtime instead of
creating an executor for every item.

To expose a reusable collection, register a fresh-stream factory once:

```csharp
IAsyncEnumerable<uint> nodes = RustEnumerable.FromFactory(tree.NodesAsync);
var first = await nodes.Take(5).ToArrayAsync();
var again = await nodes.Take(5).ToArrayAsync();
```

The async factory runs when enumeration first advances. An unstarted traversal
or `Take(0)` does not open a native stream. Each enumeration owns and disposes
its fresh stream. As with the synchronous overload, the factory determines
whether subsequent traversals observe live data or a snapshot. Cast a bare
`null` factory to its intended `Func<...>` type to select the overload.

Cancellation is terminal for that traversal. Pass a token with
`.WithCancellation(token)`, or use the active `ScriptScope` default.
The binding aborts the native task and keeps its completion context alive until
Rust acknowledges cancellation. `DisposeAsync()` waits for this acknowledgement;
`await foreach` therefore releases native state on exhaustion, `break`, or an
exception. Overlapping moves on one enumerator are rejected. A Rust stream panic
faults enumeration; cancellation produces a cancelled await.

As with other Rust futures, cancellation is cooperative: a `poll_next`
implementation that blocks a runtime worker or never returns cannot be
interrupted by the binding. Use `await using ScriptScope` to release native
sources in pipelines that never start. Outside a scope, retain and dispose the
concrete source (or use `RustEnumerable.FromFactory`).

The async stream ABI is three pointers: state, next, and drop; each next call
returns the existing `TaskHandle`. Regenerate C# and rebuild the Rust library
together when adding this pattern.

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

The synchronous API provides `Where`, `Take`, `Any()`, and `Any(predicate)`.
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


### Value tuples, deconstruction, and anonymous projections

Positional Rust FFI structs with at least two plain value fields automatically
use C# value tuples in the public Rust-library API:

```rust
#[ffi]
#[derive(Clone, Copy)]
pub struct Pair(pub u32, pub f32);

#[ffi]
pub fn echo(x: Pair) -> Pair { x }
```

```csharp
var (id, weight) = Interop.echo((17u, 2.5f));
Func<(uint, float), (uint, float)> transform = x => (x.Item1, x.Item2 * 2);
```

The same projection applies to named `callback!` Func/Action signatures,
service arguments/results, `Task<T>` results, native synchronous/asynchronous
iterator elements, and tuple payloads inside generated enum/Option/Result
cases. Nested positional values become nested tuples. Named plain value
structs keep their names and gain constructors and `Deconstruct` methods:

```csharp
var point = new Vec3f32(1, 2, 3);
var (x, y, z) = point;

using var pairs = service.Pairs();
var rows = pairs.Select(pair => new { Id = pair.Item1, Weight = pair.Item2 }).ToArray();
```

Anonymous objects in `Select` are inferred by the C# compiler and remain
managed projections. They do not require generated anonymous return types
or new Rust exports.

Eligibility is structural: primitives, plain enums with legal C# enum bases,
and structs recursively composed of these values. Single-field newtypes,
pointers, borrowed slices, strings, arrays, owning wrappers, and payload enums
keep their generated identities. Field names such as `field_0` on a named
Rust struct do not make it positional. A `Deconstruct` member is omitted
when it would collide with the type or a field name.

Each projected type retains its named native wrapper and unmanaged mirror.
Explicit field copies and a type-specific `LibraryImport` marshaller convert
between that representation and `ValueTuple`; no tuple memory is reinterpreted
as Rust memory. Reference/slice inputs still use their native wrapper types.
Bare `extern "C" fn` delegates use native mirrors for tuple-containing
structs and unions, because runtime delegate marshalling cannot use the source-generated
tuple marshaller. Use `callback!` for the convenient Func/Action API.

Union projection follows the union specification snapshot in
`D:\repos\Unions\src\Unions\UnionSpecification.md` (source commit
`f23bdbed3f5a9c5f5d78f7f2a0a9f0bc54ac58b4`): distinct Rust variants retain
distinct generated case types, even when they carry identical tuple payloads.
Case constructors, `Value`, `HasValue`, and typed `TryGetValue` remain
consistent; typed pattern matching uses the non-boxing access pattern.
Tuple projection changes the payload inside a case, not the union case identity
or native tag/layout. The generator retains the custom `[Union]` representation.

Regenerate bindings when upgrading: eligible positional signatures now use
`ValueTuple`, including invariant generic types such as `Task<T>` and
`Func<T, TResult>`. Manually constructed Rust inventories must set the new
`Struct::is_positional` flag; older serialized inventories default it to
`false`.
