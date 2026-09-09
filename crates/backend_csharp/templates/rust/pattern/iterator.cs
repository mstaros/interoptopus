/// An owned, single-pass Rust iterator. Where and Take move ownership to the
/// returned stage. Any consumes the query; dispose a query that is not consumed.
[NativeMarshalling(typeof(MarshallerMeta))]
public sealed partial class {{ name }} : global::Rust.Linq.IRustEnumerable<{{ managed_element_type }}>
{
    private readonly object _gate = new();
    private Unmanaged _native;
    private global::Rust.Linq.ScriptScope? _scope;

    private {{ name }}() { }

    private void Track(global::Rust.Linq.ScriptScope? scope)
    {
        _scope = scope;
        scope?.Register(this);
    }

    ~{{ name }}() { Dispose(); }

    internal Unmanaged IntoUnmanaged()
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_native._data == IntPtr.Zero, this);
            var native = _native;
            _native = default;
            _scope?.Unregister(this);
            GC.SuppressFinalize(this);
            return native;
        }
    }

    internal Unmanaged AsUnmanaged()
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_native._data == IntPtr.Zero, this);
            return _native;
        }
    }

    /// Releases the traversal and every retained predicate without evaluating it.
    public unsafe void Dispose()
    {
        Unmanaged native;
        lock (_gate)
        {
            native = _native;
            _native = default;
        }
        _scope?.Unregister(this);
        if (native._data != IntPtr.Zero)
            ((delegate* unmanaged[Cdecl]<IntPtr, void>)native._drop)(native._data);
        GC.SuppressFinalize(this);
    }

    internal unsafe {{ name }} WhereCore(global::System.Func<{{ managed_element_type }}, bool> predicate)
    {
        ArgumentNullException.ThrowIfNull(predicate);
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_native._data == IntPtr.Zero, this);
            var next = new {{ name }}();
            var state = new PredicateState(predicate);
            var root = GCHandle.Alloc(state);
            // Rust retains this root and releases it when the entire query is dropped.
            ((delegate* unmanaged[Cdecl]<IntPtr, IntPtr, IntPtr, IntPtr, void>)_native._where)(
                _native._data,
                (IntPtr)(delegate* unmanaged[Cdecl]<IntPtr, IntPtr, int>)&InvokePredicate,
                GCHandle.ToIntPtr(root),
                (IntPtr)(delegate* unmanaged[Cdecl]<IntPtr, void>)&ReleasePredicate);
            next._native = _native;
            _native = default;
            _scope?.Unregister(this);
            GC.SuppressFinalize(this);
            next.Track(_scope ?? global::Rust.Linq.ScriptScope.Current);
            return next;
        }
    }

    internal unsafe {{ name }} TakeCore(int count)
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_native._data == IntPtr.Zero, this);
            var next = new {{ name }}();
            ((delegate* unmanaged[Cdecl]<IntPtr, ulong, void>)_native._take)(_native._data, (ulong)Math.Max(count, 0));
            next._native = _native;
            _native = default;
            _scope?.Unregister(this);
            GC.SuppressFinalize(this);
            next.Track(_scope ?? global::Rust.Linq.ScriptScope.Current);
            return next;
        }
    }

    internal unsafe bool AnyCore()
    {
        var native = IntoUnmanaged();
        global::System.Runtime.ExceptionServices.ExceptionDispatchInfo error = null;
        int result;
        try
        {
            IntPtr context = IntPtr.Zero;
            result = ((delegate* unmanaged[Cdecl]<IntPtr, IntPtr*, int>)native._any)(native._data, &context);
            // Retrieve the exception while the native query still roots its callbacks.
            if (context != IntPtr.Zero)
                error = (GCHandle.FromIntPtr(context).Target as PredicateState)?.Error;
        }
        finally
        {
            ((delegate* unmanaged[Cdecl]<IntPtr, void>)native._drop)(native._data);
        }
        error?.Throw();
        if (result < 0) throw new InvalidOperationException("Rust iterator evaluation failed.");
        return result != 0;
    }


    global::Rust.Linq.IRustEnumerable<{{ managed_element_type }}> global::Rust.Linq.IRustEnumerable<{{ managed_element_type }}>.WhereCore(global::System.Func<{{ managed_element_type }}, bool> predicate)
        => WhereCore(predicate);
    global::Rust.Linq.IRustEnumerable<{{ managed_element_type }}> global::Rust.Linq.IRustEnumerable<{{ managed_element_type }}>.TakeCore(int count)
        => TakeCore(count);
    bool global::Rust.Linq.IRustEnumerable<{{ managed_element_type }}>.AnyCore() => AnyCore();

    /// Transfers this single-pass traversal to an enumerator without evaluating it.
    public global::System.Collections.Generic.IEnumerator<{{ managed_element_type }}> GetEnumerator()
    {
        var enumerator = new Enumerator();
        enumerator.Native = IntoUnmanaged();
        enumerator.Track(_scope ?? global::Rust.Linq.ScriptScope.Current);
        return enumerator;
    }

    global::System.Collections.IEnumerator global::System.Collections.IEnumerable.GetEnumerator() => GetEnumerator();

    private sealed class Enumerator : global::System.Collections.Generic.IEnumerator<{{ managed_element_type }}>
    {
        internal Unmanaged Native;
        private global::Rust.Linq.ScriptScope? _scope;
        internal void Track(global::Rust.Linq.ScriptScope? scope)
        {
            _scope = scope;
            scope?.Register(this);
        }
        private readonly object _gate = new();
        private bool _running;
        private bool _disposed;
        private bool _hasCurrent;
        private {{ managed_element_type }} _current;

        ~Enumerator() { Dispose(); }

        public {{ managed_element_type }} Current
        {
            get
            {
                lock (_gate)
                {
                    if (!_hasCurrent) throw new InvalidOperationException("The enumerator has no current element.");
                    return _current;
                }
            }
        }

        object global::System.Collections.IEnumerator.Current => Current;

        public unsafe bool MoveNext()
        {
            Unmanaged native;
            lock (_gate)
            {
                if (_running) throw new InvalidOperationException("Rust iterator enumeration is not reentrant.");
                if (_disposed) return false;
                _running = true;
                _hasCurrent = false;
                native = Native;
            }
            bool yielded = false;
            try
            {
                // Do not hold the monitor across user predicates. Dispose may be
                // called by the predicate itself, including from another thread.
                {{ unmanaged_element_type }} item = default;
                IntPtr context = IntPtr.Zero;
                int result = ((delegate* unmanaged[Cdecl]<IntPtr, IntPtr, IntPtr*, int>)native._next)(native._data, (IntPtr)(&item), &context);
                if (context != IntPtr.Zero)
                    (GCHandle.FromIntPtr(context).Target as PredicateState)?.Error?.Throw();
                if (result < 0) throw new InvalidOperationException("Rust iterator evaluation failed.");
                lock (_gate)
                {
                    if (_disposed || result == 0) return false;
                    _current = {{ copied_element }};
                    _hasCurrent = true;
                    yielded = true;
                    return true;
                }
            }
            finally
            {
                bool release;
                lock (_gate)
                {
                    _running = false;
                    if (!yielded) _disposed = true;
                    release = _disposed;
                }
                if (release) Dispose();
            }
        }

        public void Reset() => throw new NotSupportedException("Rust iterators are single-pass.");

        public unsafe void Dispose()
        {
            Unmanaged native;
            lock (_gate)
            {
                _disposed = true;
                _hasCurrent = false;
                _current = default;
                // Retain native state until an active MoveNext has returned from Rust.
                if (_running) return;
                native = Native;
                Native = default;
            }
            _scope?.Unregister(this);
            if (native._data != IntPtr.Zero)
                ((delegate* unmanaged[Cdecl]<IntPtr, void>)native._drop)(native._data);
            GC.SuppressFinalize(this);
        }
    }

    private sealed class PredicateState
    {
        internal readonly global::System.Func<{{ managed_element_type }}, bool> Predicate;
        internal global::System.Runtime.ExceptionServices.ExceptionDispatchInfo Error;
        internal PredicateState(global::System.Func<{{ managed_element_type }}, bool> predicate) { Predicate = predicate; }
    }

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(System.Runtime.CompilerServices.CallConvCdecl) })]
    private static unsafe int InvokePredicate(IntPtr item, IntPtr context)
    {
        PredicateState state = null;
        try
        {
            state = (PredicateState)GCHandle.FromIntPtr(context).Target;
            if (state.Error != null) return -1;
            return state.Predicate({{ read_element }}) ? 1 : 0;
        }
        catch (Exception exception)
        {
            if (state != null)
                state.Error = global::System.Runtime.ExceptionServices.ExceptionDispatchInfo.Capture(exception);
            return -1;
        }
    }

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(System.Runtime.CompilerServices.CallConvCdecl) })]
    private static void ReleasePredicate(IntPtr context)
    {
        GCHandle.FromIntPtr(context).Free();
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct Unmanaged
    {
        internal IntPtr _data;
        internal IntPtr _where;
        internal IntPtr _take;
        internal IntPtr _any;
        internal IntPtr _drop;
        internal IntPtr _next;

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
        public void FromManaged({{ name }} managed) { _managed = managed; }
        public void FromUnmanaged(Unmanaged unmanaged) { _unmanaged = unmanaged; }
        public Unmanaged ToUnmanaged() { return _managed.IntoUnmanaged(); }
        public {{ name }} ToManaged() { return _unmanaged.IntoManaged(); }
        public void Free() { }
    }

    [CustomMarshaller(typeof({{ name }}), MarshalMode.ManagedToUnmanagedIn, typeof(InMarshaller))]
    public struct InMarshallerMeta { }

    public ref struct InMarshaller
    {
        private {{ name }} _managed;
        public void FromManaged({{ name }} managed)
        {
            _managed = managed;
            global::System.Threading.Monitor.Enter(_managed._gate);
        }
        public Unmanaged ToUnmanaged() { return _managed.AsUnmanaged(); }
        public void Free() { global::System.Threading.Monitor.Exit(_managed._gate); }
    }
}

