{{ _fns_decorators_all }}
public Exception ExceptionForVariant()
{
    {%- if has_empty_state %}
    if (!_hasValue) return new InvalidOperationException("A default {{ name }} is empty and corresponds to no Rust variant. Construct one through a case constructor or factory.");
    {%- endif %}
    {%- for v in variants %}
    {%- if v.has_payload %}
    {%- for payload in v.payloads %}
    if (_variant == {{ v.id }}) return new EnumException<{{ payload.type }}>({{ payload.field }});
    {%- endfor %}
    {%- else %}
    if (_variant == {{ v.id }}) return new EnumException();
    {%- endif %}
    {%- endfor %}
    throw new InteropException("Illegal enum state detected. This is a severe error and should never happen.");
}
