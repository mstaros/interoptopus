{{ _fns_decorators_all }}
{{ _fns_decorators_internal }}
internal Unmanaged {{ to_unmanaged }}(global::System.Collections.Generic.List<Action> releases = null)
{
    {%- if has_empty_state %}
    if (!_hasValue) throw new InvalidOperationException("Cannot marshal a default {{ name }}: it is empty and corresponds to no Rust variant. Construct one through a case constructor or factory.");
    {%- endif %}
    var _unmanaged = new Unmanaged();
    {%- if to_unmanaged == "IntoUnmanaged" %}
    try
    {
    {%- endif %}
{% if to_unmanaged == "IntoUnmanaged" %}    {% endif %}    _unmanaged._variant = _variant;
    {%- for v in variants %}
    {%- for payload in v.payloads %}
{% if to_unmanaged == "IntoUnmanaged" %}    {% endif %}    if (_variant == {{ v.id }}) _unmanaged._{{ v.name }}.{{ payload.field }} = {{ payload.field }}{{ payload.to_unmanaged | replace(from=".ToUnmanaged()", to=".AsUnmanaged(releases!)") | replace(from=".IntoUnmanaged()", to=".IntoUnmanaged(releases)") }};
    {%- endfor %}
    {%- endfor %}
{% if to_unmanaged == "IntoUnmanaged" %}    {% endif %}    return _unmanaged;
    {%- if to_unmanaged == "IntoUnmanaged" %}
    }
    catch
    {
        _unmanaged.Free();
        throw;
    }
    {%- endif %}
}
