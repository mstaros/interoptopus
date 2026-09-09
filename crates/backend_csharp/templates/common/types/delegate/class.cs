[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate {{ rval_unmanaged_name }} {{ name }}Native({% for arg in args %}{{ arg.unmanaged_name }} {{ arg.name }}, {% endfor %}IntPtr callback_data);
{% if custom_delegate %}
/// Managed delegate signature for <see cref="{{ name }}"/>.
{{ visibility }} delegate {{ rval_managed }} {{ name }}Delegate({% for arg in args %}{{ arg.managed_type }} {{ arg.name }}{% if not loop.last %}, {% endif %}{% endfor %});
{% endif %}

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
delegate void {{ name }}Destructor(IntPtr data);

{{ visibility }} partial class {{ name }}
{
    private {{ managed_delegate }} _managed;
    private {{ name }}Native _native;
    private IntPtr _ptr;
    private IntPtr _data;
    private IntPtr _destructor;
    private Exception _exception;
}

/// A named callback that bridges a managed delegate and an
/// unmanaged function pointer. When created from a managed delegate, exceptions
/// thrown inside the callback are captured and re-thrown on <see cref="Dispose"/>.
///
/// When received from Rust, use <see cref="Dispose"/> after the last use to allow
/// Rust to free any associated data. When passed by value, Rust keeps the managed
/// callback alive until it drops the callback. Borrowed callbacks require the wrapper
/// to remain alive for the duration of the native call.
{{ _types_docs_owned }}
[NativeMarshalling(typeof(MarshallerMeta))]
{{ visibility }} partial class {{ name }} : IDisposable
{
    // Rust releases this rooting handle when it drops the retained callback.
    [UnmanagedCallersOnly(CallConvs = new[] { typeof(System.Runtime.CompilerServices.CallConvCdecl) })]
    private static void ReleaseHandle(IntPtr data) => GCHandle.FromIntPtr(data).Free();
    private static unsafe IntPtr ReleaseHandlePtr => (IntPtr)(delegate* unmanaged[Cdecl]<IntPtr, void>)&ReleaseHandle;

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal {{ name }}() { }

    /// Wraps a managed delegate so it can be passed to Rust as a callback.
    {{ _fns_decorators_all | indent }}
    public unsafe {{ name }}({{ managed_delegate }} managed)
    {
        ArgumentNullException.ThrowIfNull(managed);
        _managed = managed;
{% if function_pointer %}
        _ptr = (IntPtr)(delegate* unmanaged[Cdecl]<{% for arg in args %}{{ arg.unmanaged_name }}, {% endfor %}IntPtr, {{ rval_unmanaged_name }}>)&CallUnmanaged;
{% else %}
        _native = CallTrampoline;
        _ptr = Marshal.GetFunctionPointerForDelegate(_native);
{% endif %}
    }

{% if function_pointer %}
    [UnmanagedCallersOnly(CallConvs = new[] { typeof(System.Runtime.CompilerServices.CallConvCdecl) })]
    private static {{ rval_unmanaged_name }} CallUnmanaged({% for arg in args %}{{ arg.unmanaged_name }} {{ arg.name }}, {% endfor %}IntPtr callback_data)
    {
        var target = ({{ name }})GCHandle.FromIntPtr(callback_data).Target;
        {% if not is_void %}return {% endif %}target.CallTrampoline({% for arg in args %}{{ arg.name }}, {% endfor %}callback_data);
    }
{% endif %}

    {{ _fns_decorators_all | indent }}
    private {{ rval_unmanaged_name }} CallTrampoline({% for arg in args %}{{ arg.unmanaged_name }} {{ arg.name }}, {% endfor %}IntPtr callback_data)
    {
        try
        {
            {% if not is_void %}
            return {{ rval_native_prefix }}_managed({% for arg in args %}{{ arg.name }}{{ arg.to_managed }}{% if not loop.last %}, {% endif %}{% endfor %}){{ rval_native_suffix }}{{ rval_to_unmanaged }};
            {% else %}
            _managed({% for arg in args %}{{ arg.name }}{{ arg.to_managed }}{% if not loop.last %}, {% endif %}{% endfor %});
            {% endif %}
        }
        catch (Exception e)
        {
            _exception = e;
            {% if not is_void %}
            return default;
            {% else %}
            return;
            {% endif %}
        }
    }

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal unsafe {{ rval_managed }} CallRaw({% for arg in args %}{{ arg.managed_type }} {{ arg.name }}{% if not loop.last %}, {% endif %}{% endfor %})
    {
{% if function_pointer %}
        var __target = (delegate* unmanaged[Cdecl]<{% for arg in args %}{{ arg.unmanaged_name }}, {% endfor %}IntPtr, {{ rval_unmanaged_name }}>)_ptr;
{% else %}
        var __target = _native ??= Marshal.GetDelegateForFunctionPointer<{{ name }}Native>(_ptr);
{% endif %}
        {% if not is_void %}
        return __target({% for arg in args %}{{ arg.native_value }}{{ arg.to_unmanaged }}, {% endfor %}_data){{ rval_to_managed }};
        {% else %}
        __target({% for arg in args %}{{ arg.native_value }}{{ arg.to_unmanaged }}, {% endfor %}_data);
        {% endif %}
    }

    /// Invokes the callback. When created from a managed delegate, calls it directly.
    /// When received from Rust, marshals arguments and calls through the function pointer.
    {{ _fns_decorators_all | indent }}
    public {{ rval_managed }} Call({% for arg in args %}{{ arg.managed_type }} {{ arg.name }}{% if not loop.last %}, {% endif %}{% endfor %})
    {
        if (_ptr == IntPtr.Zero) throw new ObjectDisposedException(nameof({{ name }}));
        if (_managed != null)
        {
            {% if not is_void %}return {% endif %}_managed({% for arg in args %}{{ arg.name }}{% if not loop.last %}, {% endif %}{% endfor %});
            {% if is_void %}return;{% endif %}
        }
        {% if not is_void %}return {% endif %}CallRaw({% for arg in args %}{{ arg.name }}{% if not loop.last %}, {% endif %}{% endfor %});
    }

    /// Disposes the callback. If the managed delegate threw an exception during a
    /// previous invocation, that exception is re-thrown here. If the callback was
    /// received from Rust, this tells Rust to free the associated data pointer.
    {{ _fns_decorators_all | indent }}
    public unsafe void Dispose()
    {
        var exception = _exception;
        _exception = null;
        var destructor = _destructor;
        var data = _data;
        _destructor = IntPtr.Zero;
        _data = IntPtr.Zero;
        _ptr = IntPtr.Zero;
        if (destructor != IntPtr.Zero)
        {
            ((delegate* unmanaged[Cdecl]<IntPtr, void>)destructor)(data);
        }
        if (exception != null)
            System.Runtime.ExceptionServices.ExceptionDispatchInfo.Capture(exception).Throw();
    }

    /// Converts this managed callback to its unmanaged representation for passing to Rust.
    /// Roots `this` via GCHandle so the managed callback remains alive
    /// while Rust holds the function pointer. The GCHandle is freed when Rust drops the
    /// callback and invokes the destructor.
    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal Unmanaged IntoUnmanaged()
    {
        ObjectDisposedException.ThrowIf(_ptr == IntPtr.Zero, this);
        var rval = new Unmanaged();
        rval._callback = _ptr;
        if (_managed != null)
        {
            var handle = GCHandle.Alloc(this);
            rval._data = GCHandle.ToIntPtr(handle);
            rval._destructor = ReleaseHandlePtr;
        }
        else
        {
            rval._data = _data;
            rval._destructor = _destructor;
            _ptr = IntPtr.Zero;
            _data = IntPtr.Zero;
            _destructor = IntPtr.Zero;
        }
        return rval;
    }

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal Unmanaged AsUnmanaged()
    {
        ObjectDisposedException.ThrowIf(_ptr == IntPtr.Zero, this);
        var rval = new Unmanaged();
        rval._callback = _ptr;
        rval._data = _data;
        rval._destructor = _destructor;
{% if function_pointer %}
        if (_managed != null)
        {
            // Borrowed composite fields have no scoped marshaller to free a handle.
            // Lazily retain an instance thunk for that path only.
            _native ??= CallTrampoline;
            rval._callback = Marshal.GetFunctionPointerForDelegate(_native);
            rval._data = IntPtr.Zero;
            rval._destructor = IntPtr.Zero;
        }
{% endif %}
        return rval;
    }

    [CustomMarshaller(typeof({{ name }}), MarshalMode.Default, typeof(Marshaller))]
    private struct MarshallerMeta {  }

    [StructLayout(LayoutKind.Sequential)]
    public struct Unmanaged
    {
        internal IntPtr _callback;
        internal IntPtr _data;
        internal IntPtr _destructor;

        internal unsafe void Free()
        {
            var native = this;
            this = default;
            if (native._destructor != IntPtr.Zero)
                ((delegate* unmanaged[Cdecl]<IntPtr, void>)native._destructor)(native._data);
        }

        {{ _fns_decorators_all | indent(width = 8) }}
        {{ _fns_decorators_internal | indent(width = 8) }}
        internal {{ name }} IntoManaged()
        {
            var rval = new {{ name }}();
            rval._ptr = _callback;
            rval._data = _data;
            rval._destructor = _destructor;
            return rval;
        }

    }

    [CustomMarshaller(typeof({{ name }}), MarshalMode.ManagedToUnmanagedIn, typeof(InMarshaller))]
    public struct InMarshallerMeta { }

    public ref struct InMarshaller
    {
        private {{ name }} _managed;

        {{ _fns_decorators_all | indent(width = 8) }}
        public void FromManaged({{ name }} managed) { _managed = managed; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public Unmanaged ToUnmanaged() { return _managed.AsUnmanaged(); }

        {{ _fns_decorators_all | indent(width = 8) }}
        public void Free() {}
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
