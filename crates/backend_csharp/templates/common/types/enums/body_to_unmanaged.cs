{{ _fns_decorators_all }}
{{ _fns_decorators_internal }}
internal Unmanaged {{ to_unmanaged }}()
{
    {%- if has_empty_state %}
    if (!_hasValue) throw new InvalidOperationException("Cannot marshal a default {{ name }}: it is empty and corresponds to no Rust variant. Construct one through a case constructor or factory.");
    {%- endif %}
    var _unmanaged = new Unmanaged();
    _unmanaged._variant = _variant;
    {%- for v in variants %}
    {%- for payload in v.payloads %}
    if (_variant == {{ v.id }}) _unmanaged._{{ v.name }}._{{ payload.name }} = _{{ payload.name }}{{ payload.to_unmanaged }};
    {%- endfor %}
    {%- endfor %}
    return _unmanaged;
}
