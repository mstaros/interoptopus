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

    /// Creates a new Rust-owned vector by copying elements from the given span.
    {{ _fns_decorators_all | indent }}
    public static unsafe {{ name }} From(Span<{{ element_type }}> _data)
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

    /// A view over the Rust-owned memory, without copying.
    ///
    /// The span is only valid until <see cref="Dispose"/> is called, and must
    /// not outlive this instance. Use <see cref="ToArray"/> if the data needs
    /// to survive the vector.
    public unsafe ReadOnlySpan<{{ element_type }}> AsSpan()
    {
        if (_ptr == IntPtr.Zero) throw new NullReferenceException();
        return new ReadOnlySpan<{{ element_type }}>((void*)_ptr, (int)_len);
    }

    /// Copies all elements into a new managed array.
    ///
    /// Prefer this over looping the indexer: the indexer marshals one element
    /// per call, which is O(n) interop calls for what is a single copy.
    public unsafe {{ element_type }}[] ToArray()
    {
        if (_ptr == IntPtr.Zero) throw new NullReferenceException();
        if (_len == 0) return [];
        return AsSpan().ToArray();
    }

    /// Gets the element at the given index.
    public unsafe {{ element_type }} this[int i]
    {
        {{ _fns_decorators_all | indent(width = 8) }}
        get
        {
            if (_ptr == IntPtr.Zero) throw new NullReferenceException();
            if (i < 0 || (ulong)i >= _len) throw new IndexOutOfRangeException();
            return Marshal.PtrToStructure<{{ element_type }}>(new IntPtr(_ptr.ToInt64() + i * sizeof({{ element_type }})));
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
