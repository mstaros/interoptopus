/// An owned, single-pass native async stream. Use await foreach, await using,
/// or a ScriptScope to release unfinished traversals.
[NativeMarshalling(typeof(MarshallerMeta))]
public sealed partial class {{ name }} : global::System.Collections.Generic.IAsyncEnumerable<{{ managed_element_type }}>, IDisposable, IAsyncDisposable
{
    private readonly object _gate = new();
    private int __borrows;

    private void __ReleaseBorrow() { lock (_gate) { --__borrows; } }
    private Unmanaged _native;
    private global::Rust.Linq.ScriptScope? _scope;

    private {{ name }}() { }
    ~{{ name }}() { Dispose(); }

    private void Track(global::Rust.Linq.ScriptScope? scope)
    {
        _scope = scope;
        scope?.Register(this);
    }

    internal Unmanaged IntoUnmanaged(global::System.Collections.Generic.List<Action> releases = null)
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_native._data == IntPtr.Zero, this);
            if (__borrows != 0) throw new InvalidOperationException("Cannot use or transfer an iterator while it is borrowed.");
            var native = _native;
            _native = default;
            _scope?.Unregister(this);
            GC.SuppressFinalize(this);
            return native;
        }
    }

    internal Unmanaged AsUnmanaged(global::System.Collections.Generic.List<Action> releases)
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_native._data == IntPtr.Zero, this);
            __borrows = checked(__borrows + 1);
            try { releases.Add(__ReleaseBorrow); }
            catch { --__borrows; throw; }
            return _native;
        }
    }

    public unsafe void Dispose()
    {
        Unmanaged native;
        lock (_gate)
        {
            if (__borrows != 0) throw new InvalidOperationException("Cannot dispose an iterator while it is borrowed.");
            native = _native;
            _native = default;
        }
        _scope?.Unregister(this);
        Drop(native);
        GC.SuppressFinalize(this);
    }

    public ValueTask DisposeAsync() { Dispose(); return ValueTask.CompletedTask; }

    private static unsafe void Drop(Unmanaged native)
    {
        if (native._data != IntPtr.Zero)
            ((delegate* unmanaged[Cdecl]<IntPtr, void>)native._drop)(native._data);
    }

    public global::System.Collections.Generic.IAsyncEnumerator<{{ managed_element_type }}> GetAsyncEnumerator(CancellationToken cancellationToken = default)
    {
        cancellationToken = global::Rust.Linq.ScriptScope.ResolveCancellation(cancellationToken);
        if (cancellationToken.IsCancellationRequested)
        {
            Dispose();
            cancellationToken.ThrowIfCancellationRequested();
        }
        var enumerator = new Enumerator(cancellationToken);
        enumerator.Native = IntoUnmanaged();
        enumerator.Track(_scope ?? global::Rust.Linq.ScriptScope.Current);
        return enumerator;
    }

    private sealed class Request : global::System.Threading.Tasks.Sources.IValueTaskSource<(int Status, {{ managed_element_type }} Item)>
    {
        internal global::System.Threading.Tasks.Sources.ManualResetValueTaskSourceCore<(int Status, {{ managed_element_type }} Item)> Completion =
            new() { RunContinuationsAsynchronously = true };
        internal ValueTask<(int Status, {{ managed_element_type }} Item)> Next()
        {
            Completion.Reset();
            return new(this, Completion.Version);
        }
        public (int Status, {{ managed_element_type }} Item) GetResult(short token) => Completion.GetResult(token);
        public global::System.Threading.Tasks.Sources.ValueTaskSourceStatus GetStatus(short token) => Completion.GetStatus(token);
        public void OnCompleted(Action<object?> continuation, object? state, short token, global::System.Threading.Tasks.Sources.ValueTaskSourceOnCompletedFlags flags)
            => Completion.OnCompleted(continuation, state, token, flags);
        private IntPtr _root;

        internal IntPtr Root()
        {
            _root = GCHandle.ToIntPtr(GCHandle.Alloc(this));
            return _root;
        }

        internal void ReleaseRoot()
        {
            var root = Interlocked.Exchange(ref _root, IntPtr.Zero);
            if (root != IntPtr.Zero) GCHandle.FromIntPtr(root).Free();
        }
    }

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(System.Runtime.CompilerServices.CallConvCdecl) })]
    private static unsafe void Complete(IntPtr context, int status, IntPtr item)
    {
        var request = (Request)GCHandle.FromIntPtr(context).Target!;
        {{ managed_element_type }} value = default;
        Exception? error = null;
        try
        {
            if (status == 1) value = {{ read_element }};
        }
        catch (Exception exception) { error = exception; }
        // Release before waking managed continuations; the local keeps Request alive.
        request.ReleaseRoot();
        if (error is null) request.Completion.SetResult((status, value));
        else request.Completion.SetException(error);
    }

    private sealed class Enumerator : global::System.Collections.Generic.IAsyncEnumerator<{{ managed_element_type }}>, IDisposable
    {
        internal Unmanaged Native;
        private readonly object _gate = new();
        private readonly CancellationToken _token;
        private readonly TaskCompletionSource _closed = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private global::Rust.Linq.ScriptScope? _scope;
        private TaskHandle _handle;
        private readonly Request _request = new();
        private bool _moving;
        private bool _disposed;
        private bool _released;
        private bool _hasCurrent;
        private {{ managed_element_type }} _current;

        internal Enumerator(CancellationToken token) { _token = token; }
        ~Enumerator() { Dispose(); }

        internal void Track(global::Rust.Linq.ScriptScope? scope)
        {
            _scope = scope;
            scope?.Register(this);
        }

        public {{ managed_element_type }} Current
        {
            get
            {
                lock (_gate)
                {
                    if (!_hasCurrent) throw new InvalidOperationException("The async enumerator has no current element.");
                    return _current;
                }
            }
        }

        public ValueTask<bool> MoveNextAsync()
        {
            if (_token.IsCancellationRequested)
            {
                Dispose();
                return ValueTask.FromCanceled<bool>(_token);
            }
            ValueTask<(int Status, {{ managed_element_type }} Item)> completion;
            CancellationTokenRegistration registration = default;
            try
            {
                lock (_gate)
                {
                    if (_moving) throw new InvalidOperationException("Concurrent async stream advancement is not supported.");
                    if (_disposed) return ValueTask.FromResult(false);
                    _hasCurrent = false;
                    completion = _request.Next();
                    var root = _request.Root();
                    try
                    {
                        // Register before native work; failure here cannot strand a request.
                        registration = _token.UnsafeRegister(static state => ((Enumerator)state!).Abort(), this);
                    }
                    catch { _request.ReleaseRoot(); throw; }
                    _moving = true;
                    try
                    {
                        unsafe
                        {
                            _handle = ((delegate* unmanaged[Cdecl]<IntPtr, IntPtr, IntPtr, TaskHandle>)Native._next)(
                                Native._data, (IntPtr)(delegate* unmanaged[Cdecl]<IntPtr, int, IntPtr, void>)&Complete, root);
                        }
                        if (_token.IsCancellationRequested) _handle.Abort();
                    }
                    catch
                    {
                        _request.ReleaseRoot();
                        _moving = false;
                        _disposed = true;
                        throw;
                    }
                }
            }
            catch
            {
                registration.Dispose();
                FinishDispose();
                throw;
            }
            return MoveNextCoreAsync(completion, registration);
        }

        // Skip the async state machine entirely when Rust completes inside the next call.
        private ValueTask<bool> MoveNextCoreAsync(ValueTask<(int Status, {{ managed_element_type }} Item)> completion, CancellationTokenRegistration registration)
        {
            if (!completion.IsCompleted) return AwaitMoveAsync(completion, registration);
            bool yielded = false;
            try
            {
                yielded = ReadResult(completion.GetAwaiter().GetResult());
                return ValueTask.FromResult(yielded);
            }
            catch (Exception error) { return FailedMoveAsync(error); }
            finally { FinishMove(registration, yielded); }
        }

        [global::System.Runtime.CompilerServices.AsyncMethodBuilder(typeof(global::System.Runtime.CompilerServices.PoolingAsyncValueTaskMethodBuilder<>))]
        private async ValueTask<bool> AwaitMoveAsync(ValueTask<(int Status, {{ managed_element_type }} Item)> completion, CancellationTokenRegistration registration)
        {
            bool yielded = false;
            try
            {
                yielded = ReadResult(await completion.ConfigureAwait(false));
                return yielded;
            }
            finally { FinishMove(registration, yielded); }
        }

        // Async completion preserves OperationCanceledException as cancellation, including its original token.
        private static async ValueTask<bool> FailedMoveAsync(Exception error)
        {
            await ValueTask.CompletedTask;
            global::System.Runtime.ExceptionServices.ExceptionDispatchInfo.Capture(error).Throw();
            return false;
        }

        private bool ReadResult((int Status, {{ managed_element_type }} Item) result)
        {
            _token.ThrowIfCancellationRequested();
            if (result.Status == -2) throw new OperationCanceledException("The native async stream was cancelled.", _token);
            if (result.Status != 0 && result.Status != 1) throw new InvalidOperationException("Native async stream advancement failed.");
            lock (_gate)
            {
                if (_disposed || result.Status == 0) return false;
                _current = result.Item;
                _hasCurrent = true;
                return true;
            }
        }

        private void FinishMove(CancellationTokenRegistration registration, bool yielded)
        {
            // Never hold _gate while waiting for an in-flight token callback.
            registration.Dispose();
            TaskHandle handle;
            lock (_gate)
            {
                handle = _handle;
                _handle = default;
                _moving = false;
                if (!yielded) _disposed = true;
            }
            handle.Dispose();
            FinishDispose();
        }

        private void Abort()
        {
            lock (_gate)
            {
                if (_moving) _handle.Abort();
            }
        }

        public void Dispose()
        {
            lock (_gate)
            {
                _disposed = true;
                _hasCurrent = false;
                _current = default;
                if (_moving) _handle.Abort();
            }
            FinishDispose();
        }

        public ValueTask DisposeAsync()
        {
            Dispose();
            return new ValueTask(_closed.Task);
        }

        private void FinishDispose()
        {
            Unmanaged native;
            lock (_gate)
            {
                if (!_disposed || _moving || _released) return;
                _released = true;
                native = Native;
                Native = default;
                _hasCurrent = false;
                _current = default;
            }
            try { Drop(native); }
            finally
            {
                _scope?.Unregister(this);
                GC.SuppressFinalize(this);
                _closed.TrySetResult();
            }
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct Unmanaged
    {
        internal IntPtr _data;
        internal IntPtr _next;
        internal IntPtr _drop;

        internal unsafe void Free()
        {
            var native = this;
            this = default;
            Drop(native);
        }

        internal {{ name }} IntoManaged()
        {
            var managed = new {{ name }} { _native = this };
            managed.Track(global::Rust.Linq.ScriptScope.Current);
            return managed;
        }
    }

    [CustomMarshaller(typeof({{ name }}), MarshalMode.Default, typeof(Marshaller))]
    private struct MarshallerMeta { }

    public ref struct Marshaller
    {
        private {{ name }} _managed;
        private Unmanaged _unmanaged;
        private bool _ownsUnmanaged;
        public void FromManaged({{ name }} managed) { _managed = managed; }
        public void FromUnmanaged(Unmanaged unmanaged) { _unmanaged = unmanaged; }
        public Unmanaged ToUnmanaged()
        {
            _unmanaged = _managed.IntoUnmanaged();
            _ownsUnmanaged = true;
            return _unmanaged;
        }
        public {{ name }} ToManaged() => _unmanaged.IntoManaged();
        public void OnInvoked() { _ownsUnmanaged = false; }

        public void Free()
        {
            if (!_ownsUnmanaged) return;
            _ownsUnmanaged = false;
            _unmanaged.Free();
        }
    }

    [CustomMarshaller(typeof({{ name }}), MarshalMode.ManagedToUnmanagedIn, typeof(InMarshaller))]
    public struct InMarshallerMeta { }

    public ref struct InMarshaller
    {
        private {{ name }} _managed;
        private global::System.Collections.Generic.List<Action> _releases;
        public void FromManaged({{ name }} managed)
        {
            _managed = managed;
        }
        public Unmanaged ToUnmanaged() { return _managed.AsUnmanaged(_releases ??= new()); }
        public void Free()
        {
            if (_releases == null) return;
            for (var i = _releases.Count - 1; i >= 0; --i) _releases[i]();
            _releases.Clear();
        }
    }
}
