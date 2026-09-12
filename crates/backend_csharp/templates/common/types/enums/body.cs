{%- if is_union_projected -%}
[Union]
{% endif -%}
{%- if not is_managed_only -%}
[NativeMarshalling(typeof(MarshallerMeta))]
{% endif -%}
{{ visibility }} partial {{ struct_or_class }} {{ name }}{% if is_union_projected or is_result or is_disposable %} : {% if is_union_projected %}IUnion{% if is_result or is_disposable %}, {% endif %}{% endif %}{% if is_result %}IResult<{{ result_ok_name }}, {{ result_err_name }}>{% if is_disposable %}, {% endif %}{% endif %}{% if is_disposable %}IDisposable{% endif %}{% endif %}
{
{%- for item in case_types %}
    {{ item | indent }}
{% endfor -%}
{%- if not is_managed_only %}
    {%- for item in unmanaged_variants %}
    {{ item | indent }}
    {% endfor %}

    {{ unmanaged | indent }}

    {{ to_unmanaged | indent }}

    {{ as_unmanaged | indent }}
{% endif %}
    {{ exception_for_variant | indent }}
{%- if union_members %}

    {{ union_members | indent }}
{% endif -%}
{% if struct_or_class == "class" %}
    /// <summary>
    /// Private so that <c>new {{ name }}()</c> cannot produce a variant-zero instance from
    /// outside the type. Every non-null instance of a class-backed enum is valid, and this is
    /// what keeps that true. Factories, case constructors and <c>Unmanaged.ToManaged()</c>
    /// construct from inside the type and are unaffected.
    /// </summary>
    private {{ name }}() { }
{% endif %}
    {{ ctors | indent }}
{% if from_call %}
    {{ from_call | indent }}
{% endif %}
    {{ to_string | indent }}
{% if result_has_unit_methods -%}
{% if result_ok_is_unit %}
    Unit IResult<{{ result_ok_name }}, {{ result_err_name }}>.AsOk()
    {
        AsOk();
        return Unit.Default;
    }
{% endif -%}
{% if result_err_is_unit %}
    Unit IResult<{{ result_ok_name }}, {{ result_err_name }}>.AsErr()
    {
        AsErr();
        return Unit.Default;
    }
{% endif -%}
{% endif %}
{% if is_disposable %}
    public void Dispose()
    {
        {%- if has_empty_state %}
        // A default value holds no variant, so there is nothing to free. Without this the
        // dispatch below matches variant zero and survives only because `?.` short-circuits on
        // a null payload — correct by accident rather than by construction.
        if (!_hasValue) return;
        {%- endif %}
        {{ dispose_body | indent(width = 8) }}
    }
{% endif -%}
{%- if not is_managed_only %}
    [CustomMarshaller(typeof({{ name }}), MarshalMode.Default, typeof(Marshaller))]
    private struct MarshallerMeta { }

    [CustomMarshaller(typeof({{ name }}), MarshalMode.ManagedToUnmanagedIn, typeof(InMarshaller))]
    public struct InMarshallerMeta { }

    public ref struct InMarshaller
    {
        private {{ name }} _managed;
        private global::System.Collections.Generic.List<Action> _releases;

        {{ _fns_decorators_all | indent(width = 8) }}
        public void FromManaged({{ name }} managed) { _managed = managed; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public Unmanaged ToUnmanaged() { {% if rejects_null %}if (_managed is null) throw new InvalidOperationException("Cannot marshal a null {{ name }}: it corresponds to no Rust variant. Construct it through a case constructor or factory."); {% endif %}return _managed.AsUnmanaged(_releases ??= new()); }

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
        private global::System.Collections.Generic.List<Action> _releases;
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
        public Unmanaged ToUnmanaged() { {% if rejects_null %}if (_managed is null) throw new InvalidOperationException("Cannot marshal a null {{ name }}: it corresponds to no Rust variant. Construct it through a case constructor or factory."); {% endif %}{% if marshaller_to_unmanaged == "IntoUnmanaged" %}_unmanaged = _managed.IntoUnmanaged(_releases ??= new()); _ownsUnmanaged = true; return _unmanaged;{% else %}return _managed.AsUnmanaged(_releases ??= new());{% endif %} }

        {{ _fns_decorators_all | indent(width = 8) }}
        public {{ name }} ToManaged() { return _unmanaged.{{ marshaller_to_managed }}(); }

        {{ _fns_decorators_all | indent(width = 8) }}
{% if marshaller_to_unmanaged == "IntoUnmanaged" %}        public void OnInvoked() { _ownsUnmanaged = false; }

        public void Free()
        {
            try
            {
                if (_ownsUnmanaged) { _ownsUnmanaged = false; _unmanaged.Free(); }
            }
            finally
            {
                if (_releases != null)
                {
                    for (var i = _releases.Count - 1; i >= 0; --i) _releases[i]();
                    _releases.Clear();
                }
            }
        }
{% else %}        public void Free()
        {
            if (_releases == null) return;
            for (var i = _releases.Count - 1; i >= 0; --i) _releases[i]();
            _releases.Clear();
        }
{% endif %}    }

{% endif -%}
}
