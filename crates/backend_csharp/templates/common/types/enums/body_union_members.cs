// Union members
{%- if is_struct %}
public bool HasValue => _hasValue;
{%- else %}
public bool HasValue => true;
{%- endif %}

public object? Value =>
{%- if is_struct %} !_hasValue ? null :{% endif %} _variant switch
{
    {%- for v in variants %}
    {{ v.tag }} => {% if v.is_constant %}{{ v.case_type }}.{{ v.constant_member }}{% else %}new {{ v.case_type }}({% for payload in v.payloads %}{{ payload.field }}{% if not loop.last %}, {% endif %}{% endfor %}){% endif %},
    {%- endfor %}
    _ => null,
};
{%- if constants_type %}

public bool TryGetValue(out {{ constants_type }} value)
{
    if ({% if is_struct %}_hasValue && {% endif %}(_variant is {% for v in constants %}{{ v.tag }}{% if not loop.last %} or {% endif %}{% endfor %}))
    {
        value = ({{ constants_type }})_variant;
        return true;
    }
    value = default;
    return false;
}
{%- endif %}
{%- for v in variants %}
{%- if not v.is_constant %}

public bool TryGetValue(out {{ v.case_type }} value)
{
    if ({% if is_struct %}_hasValue && {% endif %}_variant == {{ v.tag }}) { value = new {{ v.case_type }}({% for payload in v.payloads %}{{ payload.field }}{% if not loop.last %}, {% endif %}{% endfor %}); return true; }
    value = default;
    return false;
}
{%- endif %}
{%- endfor %}
