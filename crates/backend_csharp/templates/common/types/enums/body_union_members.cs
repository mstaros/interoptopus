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
    {{ v.tag }} => new {{ v.case_type }}({% if v.has_payload %}_{{ v.stem }}{% endif %}),
    {%- endfor %}
    _ => null,
};
{%- for v in variants %}

public bool TryGetValue(out {{ v.case_type }} value)
{
    if ({% if is_struct %}_hasValue && {% endif %}_variant == {{ v.tag }}) { value = new {{ v.case_type }}({% if v.has_payload %}_{{ v.stem }}{% endif %}); return true; }
    value = default;
    return false;
}
{%- endfor %}
