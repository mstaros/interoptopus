public partial class {{ name }}
{
    {{ _fns_decorators_all | indent }}
    private {{ name }}() { }

    // Positive values are active borrows; -1 means disposed or transferred.
    private int __borrowState;

    private void __AcquireBorrow()
    {
        while (true)
        {
            var state = global::System.Threading.Volatile.Read(ref __borrowState);
            ObjectDisposedException.ThrowIf(state < 0, this);
            if (global::System.Threading.Interlocked.CompareExchange(ref __borrowState, checked(state + 1), state) == state) return;
        }
    }

    private void __ReleaseBorrow() => global::System.Threading.Interlocked.Decrement(ref __borrowState);

    private void __Borrow(global::System.Collections.Generic.List<Action> releases)
    {
        ArgumentNullException.ThrowIfNull(releases);
        __AcquireBorrow();
        try { releases.Add(__ReleaseBorrow); }
        catch { __ReleaseBorrow(); throw; }
    }

    private bool __Close(bool disposing)
    {
        var state = global::System.Threading.Interlocked.CompareExchange(ref __borrowState, -1, 0);
        if (state > 0) throw new InvalidOperationException("Cannot dispose or transfer this value while it is borrowed by a call or slice.");
        if (state < 0 && !disposing) throw new ObjectDisposedException(GetType().Name);
        return state == 0;
    }


    /// Creates an empty Rust-owned vector.
    {{ _fns_decorators_all | indent }}
    public static unsafe {{ name }} Empty()
    {
        InteropHelper.interoptopus_vec_create(IntPtr.Zero, 0, out var _out);
        return _out.IntoManaged();
    }

    /// The number of elements in this vector.
    public int Count
    {
        {{ _fns_decorators_all | indent(width = 8) }}
        get { __AcquireBorrow(); try { return checked((int)_len); } finally { __ReleaseBorrow(); } }
    }

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal Unmanaged IntoUnmanaged(global::System.Collections.Generic.List<Action> releases = null)
    {
        __Close(disposing: false);
        var rval = new Unmanaged();
        rval._len = _len;
        rval._capacity = _capacity;
        rval._ptr = _ptr;
        _ptr = IntPtr.Zero;
        _len = 0;
        _capacity = 0;
        return rval;
    }

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal Unmanaged AsUnmanaged(global::System.Collections.Generic.List<Action> releases)
    {
        __Borrow(releases);
        var rval = new Unmanaged();
        rval._len = _len;
        rval._capacity = _capacity;
        rval._ptr = _ptr;
        return rval;
    }

    /// Frees the underlying Rust allocation. Safe to call multiple times.
    {{ _fns_decorators_all | indent }}
    public void Dispose()
    {
        if (!__Close(disposing: true)) return;
        var _unmanaged = new Unmanaged();
        _unmanaged._ptr = _ptr;
        _unmanaged._len = _len;
        _unmanaged._capacity = _capacity;
        _ptr = IntPtr.Zero;
        _len = 0;
        _capacity = 0;
        _unmanaged.Free();
    }

    {{ _fns_decorators_all | indent }}
    public override string ToString()
    {
        return "{{ name }} { ... }";
    }

    internal partial class InteropHelper
    {
        [LibraryImport(Interop.NativeLib, EntryPoint = "{{ create_entry_point }}")]
        [UnmanagedCallConv(CallConvs = new[] { typeof(System.Runtime.CompilerServices.CallConvCdecl) })]
        internal static partial long interoptopus_vec_create(IntPtr vec, ulong len, out Unmanaged rval);
        [LibraryImport(Interop.NativeLib, EntryPoint = "{{ destroy_entry_point }}")]
        [UnmanagedCallConv(CallConvs = new[] { typeof(System.Runtime.CompilerServices.CallConvCdecl) })]
        internal static partial long interoptopus_vec_destroy(Unmanaged vec);
    }

    [CustomMarshaller(typeof({{ name }}), MarshalMode.Default, typeof(Marshaller))]
    private struct MarshallerMeta { }

    [StructLayout(LayoutKind.Sequential)]
    public struct Unmanaged
    {
        internal IntPtr _ptr;
        internal ulong _len;
        internal ulong _capacity;

        internal void Free()
        {
            var native = this;
            this = default;
            if (native._ptr != IntPtr.Zero) InteropHelper.interoptopus_vec_destroy(native);
        }

        {{ _fns_decorators_all | indent(width = 8) }}
        {{ _fns_decorators_internal | indent(width = 8) }}
        internal {{ name }} IntoManaged()
        {
            var rval = new {{ name }}();
            rval._len = _len;
            rval._capacity = _capacity;
            rval._ptr = _ptr;
            return rval;
        }
    }

    [CustomMarshaller(typeof({{ name }}), MarshalMode.ManagedToUnmanagedIn, typeof(InMarshaller))]
    public struct InMarshallerMeta { }

    public ref struct InMarshaller
    {
        private {{ name }} _managed;
        private global::System.Collections.Generic.List<Action> _releases;

        {{ _fns_decorators_all | indent(width = 8) }}
        public void FromManaged({{ name }} managed) { _managed = managed; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public Unmanaged ToUnmanaged() { return _managed.AsUnmanaged(_releases ??= new()); }

        {{ _fns_decorators_all | indent(width = 8) }}
        public void Free()
        {
            if (_releases == null) return;
            for (var i = _releases.Count - 1; i >= 0; --i) _releases[i]();
            _releases.Clear();
        }
    }

    public ref struct Marshaller
    {
        private {{ name }} _managed;
        private Unmanaged _unmanaged;
        private bool _ownsUnmanaged;

        {{ _fns_decorators_all | indent(width = 8) }}
        public Marshaller({{ name }} managed) { _managed = managed; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public Marshaller(Unmanaged unmanaged) { _unmanaged = unmanaged; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public void FromManaged({{ name }} managed) { _managed = managed; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public void FromUnmanaged(Unmanaged unmanaged) { _unmanaged = unmanaged; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public Unmanaged ToUnmanaged() { _unmanaged = _managed.IntoUnmanaged();
            _ownsUnmanaged = true;
            return _unmanaged; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public {{ name }} ToManaged() { return _unmanaged.IntoManaged(); }

        {{ _fns_decorators_all | indent(width = 8) }}
        public void OnInvoked() { _ownsUnmanaged = false; }

        public void Free()
        {
            if (!_ownsUnmanaged) return;
            _ownsUnmanaged = false;
            _unmanaged.Free();
        }
    }
}
