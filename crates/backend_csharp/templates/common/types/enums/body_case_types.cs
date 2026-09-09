{%- if is_constants -%}
public enum {{ case_type }} : {{ discriminant_type }}
{
    {%- for v in constants %}
    {%- if v.docs %}
    {{ v.docs | indent }}
    {%- endif %}
    {{ v.name }} = {{ v.id }},
    {%- endfor %}
}
{%- elif payloads -%}
public readonly record struct {{ case_type }}({% for payload in payloads %}{{ payload.type }} {{ payload.case_field }}{% if not loop.last %}, {% endif %}{% endfor %});
{%- else -%}
public readonly record struct {{ case_type }}();
{%- endif -%}
