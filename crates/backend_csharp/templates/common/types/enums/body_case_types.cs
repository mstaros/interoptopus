{%- if payloads -%}
public readonly record struct {{ case_type }}({% for payload in payloads %}{{ payload.type }} Value{% if not loop.last %}, {% endif %}{% endfor %});
{%- else -%}
public readonly record struct {{ case_type }}();
{%- endif -%}
