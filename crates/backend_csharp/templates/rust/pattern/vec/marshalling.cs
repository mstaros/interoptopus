{%- include "rust/pattern/vec/common_fields.cs" %}

{%- include "rust/pattern/vec/common_body.cs" %}

/// A Rust-allocated growable array of <c>{{ element_type }}</c> (marshalled elements).
///
/// Elements are marshalled from their unmanaged representation on each access.
{{ _types_docs_owned }}
[NativeMarshalling(typeof(MarshallerMeta))]
public partial class {{ name }} : IDisposable
{

    /// Copies the span into a new Rust-owned vector.
    {{ _fns_decorators_all | indent }}
    public static {{ name }} From(Span<{{ element_type }}> data) => From((ReadOnlySpan<{{ element_type }}>)data);

    /// Creates a new Rust-owned vector by marshalling each element from the given span.
    {{ _fns_decorators_all | indent }}
    public static unsafe {{ name }} From(ReadOnlySpan<{{ element_type }}> _data)
    {
        var releases = new global::System.Collections.Generic.List<Action>();
        try
        {
            var _temp = new {{ unmanaged_element_type }}[_data.Length];
            for (var i = 0; i < _data.Length; ++i)
                _temp[i] = _data[i].AsUnmanaged(releases);
            fixed (void* _data_ptr = _temp)
            {
                InteropHelper.interoptopus_vec_create((IntPtr)_data_ptr, (ulong)_data.Length, out var native);
                return native.IntoManaged();
            }
        }
        finally
        {
            for (var i = releases.Count - 1; i >= 0; --i) releases[i]();
        }
    }

{% if clone_elements %}    internal partial class InteropHelper
    {
        [LibraryImport(Interop.NativeLib, EntryPoint = "{{ clone_element_entry_point }}")]
        [UnmanagedCallConv(CallConvs = new[] { typeof(System.Runtime.CompilerServices.CallConvCdecl) })]
        internal static partial long interoptopus_vec_clone_element(IntPtr source, out {{ unmanaged_element_type }} value);
    }

{% endif %}    /// Returns an independent copy of the element. Dispose owned values when finished.
    public unsafe {{ element_type }} this[int i]
    {
        {{ _fns_decorators_all | indent(width = 8) }}
        get
        {
            __AcquireBorrow();
            try
            {
            if (i < 0 || (ulong)i >= _len) throw new IndexOutOfRangeException();
{% if clone_elements %}            var _source = (IntPtr)(({{ unmanaged_element_type }}*)_ptr + i);
            if (InteropHelper.interoptopus_vec_clone_element(_source, out var _element) != 0)
                throw new InvalidOperationException("Could not clone the vector element.");
{% else %}            var _element = *(({{ unmanaged_element_type }}*)_ptr + i);
{% endif %}
                return _element.{{ element_to_managed }}();
            }
            finally { __ReleaseBorrow(); }
        }
    }
}

/// Convenience extension to convert a <c>{{ element_type }}[]</c> array to a <see cref="{{ name }}"/>.
public static class {{ name }}Extensions
{
    /// Marshals the array into a new Rust-owned <see cref="{{ name }}"/>.
    /// Call <see cref="{{ name }}.Dispose"/> if the value is not passed back to Rust.
    {{ _fns_decorators_all | indent }}
    public static {{ name }} IntoVec(this {{ element_type }}[] s) { return {{ name }}.From(s); }
}
