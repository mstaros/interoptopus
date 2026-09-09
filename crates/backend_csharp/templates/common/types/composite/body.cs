[NativeMarshalling(typeof(MarshallerMeta))]
{{ visibility }} partial {{ struct_or_class }} {{ name }}{% if is_disposable %} : IDisposable{% endif %}
{
{%- if conveniences | trim %}
    {{ conveniences | trim | indent }}
{% endif %}
    {{ unmanaged | indent }}

    {{ to_unmanaged | indent }}

    {{ as_unmanaged | indent }}
{% if is_disposable %}
    public void Dispose()
    {
        {{ dispose_body | indent(width = 8) }}
    }
{% endif %}
    {{ _fns_decorators_all | indent }}
    public override string ToString()
    {
        return "{{name}} { ... }";
    }

    [CustomMarshaller(typeof({{ name }}), MarshalMode.Default, typeof(Marshaller))]
    private struct MarshallerMeta { }
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
        {%- if marshaller_to_unmanaged == "IntoUnmanaged" %}
        private bool _ownsUnmanaged;
        {%- endif %}

        {{ _fns_decorators_all | indent(width = 8) }}
        public Marshaller({{ name }} managed) { _managed = managed; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public Marshaller(Unmanaged unmanaged) { _unmanaged = unmanaged; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public void FromManaged({{ name }} managed) { _managed = managed; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public void FromUnmanaged(Unmanaged unmanaged) { _unmanaged = unmanaged; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public Unmanaged ToUnmanaged() { {% if marshaller_to_unmanaged == "IntoUnmanaged" %}_unmanaged = _managed.IntoUnmanaged(); _ownsUnmanaged = true; return _unmanaged;{% else %}return _managed.{{ marshaller_to_unmanaged }}();{% endif %} }

        {{ _fns_decorators_all | indent(width = 8) }}
        public {{ name }} ToManaged() { return _unmanaged.{{ marshaller_to_managed }}(); }

        {{ _fns_decorators_all | indent(width = 8) }}
{% if marshaller_to_unmanaged == "IntoUnmanaged" %}        public void OnInvoked() { _ownsUnmanaged = false; }

        public void Free()
        {
            if (!_ownsUnmanaged) return;
            _ownsUnmanaged = false;
            _unmanaged.Free();
        }
{% else %}        public void Free() {}
{% endif %}    }

}
