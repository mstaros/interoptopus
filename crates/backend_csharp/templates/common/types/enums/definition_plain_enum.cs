{%- if docs %}
{{ docs }}
{%- endif %}
{{ visibility }} enum {{ name }} : {{ discriminant_type }}
{
    {%- for variant in variants %}
    {{ variant.name }} = {{ variant.tag }},
    {%- endfor %}
}