public partial class {{ name }}
{
    IntPtr _data;
    ulong _len;
    bool _ownsData;
    global::System.Collections.Generic.List<Action> _owners;
}

/// A read-only view into a contiguous region of <c>{{ element_type }}</c> elements,
/// with marshalling support for non-blittable element types.
///
/// Elements are marshalled from their unmanaged representation on each access.
/// Slices created from managed arrays own a temporary native copy. Slices received
/// from Rust borrow memory that must remain valid while the slice is used.
/// Call <see cref="Dispose"/> to release this wrapper; borrowed memory is never freed.
{{ _types_docs_owned }}
[NativeMarshalling(typeof(MarshallerMeta))]
public partial class {{ name }} : IDisposable
{
    /// The number of elements in this slice.
    public int Count { get { __AcquireBorrow(); try { return checked((int)_len); } finally { __ReleaseBorrow(); } } }

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


{% if has_indexer %}
    /// Gets the element at the given index, marshalling from its unmanaged form.
    public unsafe {{ element_type }} this[int i]
    {
        {{ _fns_decorators_all | indent(width = 8) }}
        get
        {
            __AcquireBorrow();
            try
            {
            if (i < 0 || (ulong)i >= _len) throw new IndexOutOfRangeException();
            if (_data == IntPtr.Zero) { throw new NullReferenceException(); }
            var size = Marshal.SizeOf<{{ unmanaged_element_type }}>();
            var ptr = IntPtr.Add(_data, i * size);
            var unmanaged = Marshal.PtrToStructure<{{ unmanaged_element_type }}>(ptr);
                return unmanaged.{{ element_to_managed }}();
            }
            finally { __ReleaseBorrow(); }
        }
    }
{% endif %}

    {{ _fns_decorators_all | indent }}
    {{ name }}() { }

    ~{{ name }}() { Dispose(); }

    /// Creates a slice by marshalling each element from a managed array into a native copy.
    {{ _fns_decorators_all | indent }}
    public static unsafe {{ name }} From({{ element_type }}[] managed)
    {
        ArgumentNullException.ThrowIfNull(managed);
{% if reject_null_elements %}        for (var i = 0; i < managed.Length; ++i)
        {
            if (managed[i] is null)
            {
                throw new InvalidOperationException("Cannot marshal {{ name }} element at index " + i + ": {{ element_type }} is null and corresponds to no Rust variant. Construct it through a case constructor or factory.");
            }
        }
{% endif %}        var rval = new {{ name }}();
        var size = Marshal.SizeOf<{{ unmanaged_element_type }}>();
        rval._data = Marshal.AllocHGlobal(checked(size * managed.Length));
        rval._ownsData = true;
        rval._owners = new();
        rval._len = (ulong) managed.Length;
        try
        {
            for (var i = 0; i < managed.Length; ++i)
            {
                var unmanaged = managed[i].AsUnmanaged(rval._owners);
                var dst = IntPtr.Add(rval._data, i * size);
                Marshal.StructureToPtr(unmanaged, dst, false);
            }
        }
        catch
        {
            rval.Dispose();
            throw;
        }
        return rval;
    }

    /// Frees the native copy. Safe to call multiple times.
    {{ _fns_decorators_all | indent }}
    public void Dispose()
    {
        if (!__Close(disposing: true)) return;
        if (_ownsData && _data != IntPtr.Zero) Marshal.FreeHGlobal(_data);
        _data = IntPtr.Zero;
        _len = 0;
        if (_owners != null)
        {
            for (var i = _owners.Count - 1; i >= 0; --i) _owners[i]();
            _owners.Clear();
        }
        GC.SuppressFinalize(this);
    }

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal Unmanaged ToUnmanaged()
    {
        ObjectDisposedException.ThrowIf(global::System.Threading.Volatile.Read(ref __borrowState) < 0, this);
        var unmanaged = new Unmanaged();
        unmanaged._data = _data;
        unmanaged._len = _len;
        return unmanaged;
    }

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal Unmanaged AsUnmanaged(global::System.Collections.Generic.List<Action> releases)
    {
        __Borrow(releases);
        return new Unmanaged { _data = _data, _len = _len };
    }

    [CustomMarshaller(typeof({{ name }}), MarshalMode.Default, typeof(Marshaller))]
    private struct MarshallerMeta { }

    [StructLayout(LayoutKind.Sequential)]
    public struct Unmanaged
    {
        public IntPtr _data;
        public ulong _len;

        {{ _fns_decorators_all | indent(width = 8) }}
        {{ _fns_decorators_internal | indent(width = 8) }}
        internal {{ name }} ToManaged()
        {
            var _managed = new {{ name }}();
            _managed._data = _data;
            _managed._len = _len;
            return _managed;
        }
    }
}

{%- include "rust/pattern/slice/common_marshaller.cs" %}

/// Convenience extension to convert a <c>{{ element_type }}[]</c> array to a <see cref="{{ name }}"/>.
public static class {{ name }}Extensions
{
    /// Marshals the array into a <see cref="{{ name }}"/>. Call <see cref="{{ name }}.Dispose"/> when done.
    public static {{ name }} {{ method }}(this {{ element_type }}[] s) { return {{ name }}.From(s); }
}
