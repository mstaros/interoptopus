[StructLayout(LayoutKind.Explicit)]
internal unsafe struct Unmanaged
{
    [FieldOffset(0)]
    internal {{ discriminant_type }} _variant;
    {%- for v in variants %}

    [FieldOffset(0)]
    internal Unmanaged{{ v.name }} _{{ v.name }};
    {%- endfor %}

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal {{ name }} {{ to_managed_method }}()
    {
        {%- if is_union_projected %}
        return _variant switch
        {
            {%- for v in all_variants %}
            {%- if v.has_payload %}
            {{ v.id }} => new {{ name }}(new {{ v.case_type }}(_{{ v.name }}._{{ v.name }}{{ v.to_managed }})),
            {%- else %}
            {{ v.id }} => new {{ name }}(new {{ v.case_type }}()),
            {%- endif %}
            {%- endfor %}
            _ => throw new InteropException($"Unknown variant {_variant} for {{ name }}. The value did not originate from a well-formed Rust enum."),
        };
        {%- else %}
        var _managed = new {{ name }}();
        _managed._variant = _variant;
        {%- for v in variants %}
        if (_variant == {{ v.id }}) _managed._{{ v.name }} = _{{ v.name }}._{{ v.name }}{{ v.to_managed }};
        {%- endfor %}
        return _managed;
        {%- endif %}
    }
}
