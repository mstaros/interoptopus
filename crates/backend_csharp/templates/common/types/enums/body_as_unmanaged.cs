{{ _fns_decorators_all }}
{{ _fns_decorators_internal }}
internal Unmanaged AsUnmanaged()
{
    {%- if has_empty_state %}
    if (!_hasValue) throw new InvalidOperationException("Cannot marshal a default {{ name }}: it is empty and corresponds to no Rust variant. Construct one through a case constructor or factory.");
    {%- endif %}
    var _unmanaged = new Unmanaged();
    _unmanaged._variant = _variant;
    {%- for v in variants %}
    if (_variant == {{ v.id }}) _unmanaged._{{ v.name }}._{{ v.name }} = _{{ v.name }}{{ v.as_unmanaged }};
    {%- endfor %}
    return _unmanaged;
}
