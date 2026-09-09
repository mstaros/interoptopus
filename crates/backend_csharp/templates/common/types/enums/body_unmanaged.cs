[StructLayout(LayoutKind.Explicit)]
public unsafe struct Unmanaged
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
            {%- if v.is_constant %}
            {{ v.id }} => new {{ name }}({{ v.case_type }}.{{ v.constant_member }}),
            {%- elif v.has_payload %}
            {{ v.id }} => new {{ name }}(new {{ v.case_type }}({% for payload in v.payloads %}_{{ v.name }}.{{ payload.field }}{{ payload.to_managed }}{% if not loop.last %}, {% endif %}{% endfor %})),
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
        {%- for payload in v.payloads %}
        if (_variant == {{ v.id }}) _managed.{{ payload.field }} = _{{ v.name }}.{{ payload.field }}{{ payload.to_managed }};
        {%- endfor %}
        {%- endfor %}
        return _managed;
        {%- endif %}
    }
{% if to_managed_method == "IntoManaged" %}
    // Releases native fields even when only a prefix finished marshalling.
    internal void Free()
    {
        var native = this;
        this = default;
{% for v in variants %}{% for payload in v.payloads %}{% if payload.is_owned == "true" %}        if (native._variant == {{ v.id }}) native._{{ v.name }}.{{ payload.field }}.Free();
{% endif %}{% endfor %}{% endfor %}    }
{% endif %}}
