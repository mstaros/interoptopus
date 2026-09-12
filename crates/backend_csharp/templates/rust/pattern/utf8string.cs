public partial class Utf8String
{
    IntPtr _ptr;
    ulong _len;
    ulong _capacity;
}

/// A Rust-allocated UTF-8 string.
///
/// This type wraps a native Rust <c>String</c>. Reading <see cref="String"/> decodes
/// the UTF-8 buffer and allocates a managed string.
{{ _types_docs_owned }}
[NativeMarshalling(typeof(MarshallerMeta))]
public partial class Utf8String : IDisposable
{
    private Utf8String() { }

    // Positive values are active borrows; -1 means disposed or transferred.
    private int __borrowState;

    private void __AcquireBorrow()
    {
        while (true)
        {
            var state = global::System.Threading.Volatile.Read(ref __borrowState);
            ObjectDisposedException.ThrowIf(state < 0, this);
            if (global::System.Threading.Interlocked.CompareExchange(ref __borrowState, checked(state + 1), state) != state) continue;
            if (_ptr != IntPtr.Zero) return;
            __ReleaseBorrow();
            throw new ObjectDisposedException(GetType().Name);
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
        if (!disposing && (state < 0 || _ptr == IntPtr.Zero)) throw new ObjectDisposedException(GetType().Name);
        return state == 0;
    }


    /// Creates a new Rust-owned <see cref="Utf8String"/> from a managed string.
    {{ _fns_decorators_all | indent }}
    public static unsafe Utf8String From(string s)
    {
        var rval = new Utf8String();
        var source = s.AsSpan();
        var byteCount = Encoding.UTF8.GetByteCount(source);
        byte[] rented = null;
        Span<byte> utf8Bytes = byteCount <= 1024
            ? stackalloc byte[byteCount]
            : (rented = System.Buffers.ArrayPool<byte>.Shared.Rent(byteCount)).AsSpan(0, byteCount);

        try
        {
            var len = Encoding.UTF8.GetBytes(source, utf8Bytes);
            fixed (byte* p = utf8Bytes)
            {
                InteropHelper.interoptopus_string_create((IntPtr)p, (ulong)len, out var native);
                rval._ptr = native._ptr;
                rval._len = native._len;
                rval._capacity = native._capacity;
            }
        }
        finally
        {
            if (rented != null) System.Buffers.ArrayPool<byte>.Shared.Return(rented);
        }

        return rval;
    }

    /// Creates an empty Rust-owned <see cref="Utf8String"/>.
    {{ _fns_decorators_all | indent }}
    public static unsafe Utf8String Empty()
    {
        InteropHelper.interoptopus_string_create(IntPtr.Zero, 0, out var _out);
        return _out.IntoManaged();
    }


    /// Converts the native UTF-8 buffer to a managed string, leaving the native buffer alive.
    /// The caller must still call <see cref="Dispose"/> to free the native memory.
    public unsafe string String
    {
        get
        {
            __AcquireBorrow();
            try { return Encoding.UTF8.GetString(new ReadOnlySpan<byte>((byte*)_ptr, checked((int)_len))); }
            finally { __ReleaseBorrow(); }
        }
    }

    /// Converts the native UTF-8 buffer to a managed string and disposes the native buffer.
    /// After this call the <see cref="Utf8String"/> instance is consumed and must not be used again.
    {{ _fns_decorators_all | indent }}
    public unsafe string IntoString()
    {
        var native = IntoUnmanaged();
        try { return Encoding.UTF8.GetString(new ReadOnlySpan<byte>((byte*)native._ptr, checked((int)native._len))); }
        finally { native.Free(); }
    }

    /// Frees the native Rust memory. Safe to call multiple times.
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

    /// Creates an independent copy of this string, backed by a new Rust allocation.
    {{ _fns_decorators_all | indent }}
    public Utf8String Clone()
    {
        __AcquireBorrow();
        try
        {
            var source = new Unmanaged { _ptr = _ptr, _len = _len, _capacity = _capacity };
            var copy = new Unmanaged();
            InteropHelper.interoptopus_string_clone(ref source, ref copy);
            return copy.IntoManaged();
        }
        finally { __ReleaseBorrow(); }
    }

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal Unmanaged IntoUnmanaged(global::System.Collections.Generic.List<Action> releases = null)
    {
        __Close(disposing: false);
        var _unmanaged = new Unmanaged();
        _unmanaged._ptr = _ptr;
        _unmanaged._len = _len;
        _unmanaged._capacity = _capacity;
        _ptr = IntPtr.Zero;
        _len = 0;
        _capacity = 0;
        return _unmanaged;
    }

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal Unmanaged AsUnmanaged(global::System.Collections.Generic.List<Action> releases)
    {
        __Borrow(releases);
        var _unmanaged = new Unmanaged();
        _unmanaged._ptr = _ptr;
        _unmanaged._len = _len;
        _unmanaged._capacity = _capacity;
        return _unmanaged;
    }

    [StructLayout(LayoutKind.Sequential)]
    public unsafe struct Unmanaged
    {
        public IntPtr _ptr;
        public ulong _len;
        public ulong _capacity;

        internal void Free()
        {
            var native = this;
            this = default;
            if (native._ptr != IntPtr.Zero) InteropHelper.interoptopus_string_destroy(native);
        }

        {{ _fns_decorators_all | indent(width = 8) }}
        {{ _fns_decorators_internal | indent(width = 8) }}
        internal Utf8String IntoManaged()
        {
            var _managed = new Utf8String();
            _managed._ptr = _ptr;
            _managed._len = _len;
            _managed._capacity = _capacity;
            return _managed;
        }

    }

    internal partial class InteropHelper
    {
        [LibraryImport(Interop.NativeLib, EntryPoint = "{{ create_entry_point }}")]
        [UnmanagedCallConv(CallConvs = new[] { typeof(System.Runtime.CompilerServices.CallConvCdecl) })]
        {{ _fns_decorators_all | indent(width = 8) }}

        public static partial long interoptopus_string_create(IntPtr utf8, ulong len, out Unmanaged rval);

        [LibraryImport(Interop.NativeLib, EntryPoint = "{{ destroy_entry_point }}")]

        [UnmanagedCallConv(CallConvs = new[] { typeof(System.Runtime.CompilerServices.CallConvCdecl) })]
        {{ _fns_decorators_all | indent(width = 8) }}

        public static partial long interoptopus_string_destroy(Unmanaged utf8);

        [LibraryImport(Interop.NativeLib, EntryPoint = "{{ clone_entry_point }}")]

        [UnmanagedCallConv(CallConvs = new[] { typeof(System.Runtime.CompilerServices.CallConvCdecl) })]
        {{ _fns_decorators_all | indent(width = 8) }}

        public static partial long interoptopus_string_clone(ref Unmanaged orig, ref Unmanaged cloned);
    }

    [CustomMarshaller(typeof(Utf8String), MarshalMode.Default, typeof(Marshaller))]
    private struct MarshallerMeta { }

    [CustomMarshaller(typeof(Utf8String), MarshalMode.ManagedToUnmanagedIn, typeof(InMarshaller))]
    public struct InMarshallerMeta { }

    public ref struct InMarshaller
    {
        private Utf8String _managed;
        private global::System.Collections.Generic.List<Action> _releases;

        {{ _fns_decorators_all | indent(width = 8) }}
        public void FromManaged(Utf8String managed) { _managed = managed; }

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
        private Utf8String _managed; // Used when converting managed -> unmanaged
        private Unmanaged _unmanaged;
        private bool _ownsUnmanaged; // Used when converting unmanaged -> managed

        {{ _fns_decorators_all | indent(width = 8) }}
        public Marshaller(Utf8String managed) { _managed = managed; }
        {{ _fns_decorators_all | indent(width = 8) }}
        public Marshaller(Unmanaged unmanaged) { _unmanaged = unmanaged; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public void FromManaged(Utf8String managed) { _managed = managed; }
        {{ _fns_decorators_all | indent(width = 8) }}
        public void FromUnmanaged(Unmanaged unmanaged) { _unmanaged = unmanaged; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public unsafe Unmanaged ToUnmanaged()
        {
            _unmanaged = _managed.IntoUnmanaged();
            _ownsUnmanaged = true;
            return _unmanaged;
        }

        {{ _fns_decorators_all | indent(width = 8) }}
        public unsafe Utf8String ToManaged()
        {
            return _unmanaged.IntoManaged();
        }

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


/// Convenience extension to convert a <see cref="string"/> to a <see cref="Utf8String"/>.
public static class StringExtensions
{
    /// Converts this string to a Rust-owned <see cref="Utf8String"/>.
    /// Call <see cref="Utf8String.Dispose"/> if the value is not passed back to Rust.
    public static Utf8String Utf8(this string s) { return Utf8String.From(s); }
}
