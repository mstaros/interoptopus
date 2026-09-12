{%- include "rust/pattern/vec/common_fields.cs" %}

{% include "rust/pattern/vec/common_body.cs" %}

/// A Rust-allocated growable array of <c>{{ element_type }}</c> (blittable elements).
///
/// The memory is owned by Rust. Elements can be read via the indexer, or in
/// bulk via <see cref="AsSpan"/> and <see cref="ToArray"/>.
{{ _types_docs_owned }}
[NativeMarshalling(typeof(MarshallerMeta))]
public partial class {{ name }} : IDisposable
{

    /// Copies the span into a new Rust-owned vector.
    {{ _fns_decorators_all | indent }}
    public static {{ name }} From(Span<{{ element_type }}> data) => From((ReadOnlySpan<{{ element_type }}>)data);

    /// Creates a new Rust-owned vector by copying elements from the given span.
    {{ _fns_decorators_all | indent }}
    public static unsafe {{ name }} From(ReadOnlySpan<{{ element_type }}> _data)
    {
        var rval = new {{ name }}();
        fixed (void* _data_ptr = _data)
        {
            InteropHelper.interoptopus_vec_create((IntPtr) _data_ptr, (ulong)_data.Length, out var _out);
            rval._len = _out._len;
            rval._capacity = _out._capacity;
            rval._ptr = _out._ptr;
        }
        return rval;
    }

    /// Returns a span over an independent managed copy that survives disposal.
    public ReadOnlySpan<{{ element_type }}> AsSpan() => ToArray();

    /// Copies all elements into a new managed array while holding a native borrow.
    public unsafe {{ element_type }}[] ToArray()
    {
        __AcquireBorrow();
        try { return new ReadOnlySpan<{{ element_type }}>((void*)_ptr, checked((int)_len)).ToArray(); }
        finally { __ReleaseBorrow(); }
    }

    /// Gets the element at the given index.
    public unsafe {{ element_type }} this[int i]
    {
        {{ _fns_decorators_all | indent(width = 8) }}
        get
        {
            __AcquireBorrow();
            try
            {
                if (i < 0 || (ulong)i >= _len) throw new IndexOutOfRangeException();
                return (({{ element_type }}*)_ptr)[i];
            }
            finally { __ReleaseBorrow(); }
        }
    }
}

/// Convenience extension to convert a <c>{{ element_type }}[]</c> array to a <see cref="{{ name }}"/>.
public static class {{ name }}Extensions
{
    /// Copies the array into a new Rust-owned <see cref="{{ name }}"/>.
    /// Call <see cref="{{ name }}.Dispose"/> if the value is not passed back to Rust.
    public static {{ name }} Vec(this {{ element_type }}[] s) { return {{ name }}.From(s); }
}
