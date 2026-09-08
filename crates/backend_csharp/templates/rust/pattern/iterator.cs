/// An owned, single-pass Rust iterator. Where and Take move ownership to the
/// returned stage. Any consumes the query; dispose a query that is not consumed.
[NativeMarshalling(typeof(MarshallerMeta))]
public sealed partial class {{ name }} : IDisposable
{
    private readonly object _gate = new();
    private Unmanaged _native;

    private {{ name }}() { }

    ~{{ name }}() { Dispose(); }

    internal Unmanaged IntoUnmanaged()
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_native._data == IntPtr.Zero, this);
            var native = _native;
            _native = default;
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
            GC.SuppressFinalize(this);
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
            GC.SuppressFinalize(this);
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

        internal {{ name }} IntoManaged() => new {{ name }} { _native = this };
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

/// Query operators backed by the Rust traversal. Each operation consumes its receiver.
public static class {{ name }}Extensions
{
    public static {{ name }} Where(this {{ name }} source, global::System.Func<{{ managed_element_type }}, bool> predicate)
    {
        ArgumentNullException.ThrowIfNull(source);
        return source.WhereCore(predicate);
    }

    public static {{ name }} Take(this {{ name }} source, int count)
    {
        ArgumentNullException.ThrowIfNull(source);
        return source.TakeCore(count);
    }

    public static bool Any(this {{ name }} source)
    {
        ArgumentNullException.ThrowIfNull(source);
        return source.AnyCore();
    }

    public static bool Any(this {{ name }} source, global::System.Func<{{ managed_element_type }}, bool> predicate)
    {
        return source.Where(predicate).Any();
    }
}
