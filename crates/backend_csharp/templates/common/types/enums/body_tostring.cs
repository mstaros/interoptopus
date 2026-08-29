{{ _fns_decorators_all }}
public override string ToString()
{
    {%- if writes_has_value %}
    if (!_hasValue) return "<empty>";
    {%- endif %}
    {%- for v in variants %}
    {%- if v.has_payload %}
    if (_variant == {{ v.id }}) return "{{ v.name }}(...)";
    {%- else %}
    if (_variant == {{ v.id }}) return "{{ v.name }}";
    {%- endif %}
    {%- endfor %}
    throw new InteropException("Illegal enum state detected. This is a severe error and should never happen.");
}
