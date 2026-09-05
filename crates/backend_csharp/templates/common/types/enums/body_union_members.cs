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
    {{ v.tag }} => new {{ v.case_type }}({% for payload in v.payloads %}{{ payload.field }}{% if not loop.last %}, {% endif %}{% endfor %}),
    {%- endfor %}
    _ => null,
};
{%- for v in variants %}

public bool TryGetValue(out {{ v.case_type }} value)
{
    if ({% if is_struct %}_hasValue && {% endif %}_variant == {{ v.tag }}) { value = new {{ v.case_type }}({% for payload in v.payloads %}{{ payload.field }}{% if not loop.last %}, {% endif %}{% endfor %}); return true; }
    value = default;
    return false;
}
{%- endfor %}
