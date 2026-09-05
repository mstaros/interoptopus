{%- if docs %}
{{ docs }}
{%- endif %}
{%- if is_disposable %}
{{ _types_docs_owned }}
{%- endif %}
{{ visibility }} partial {{ struct_or_class }} {{ name }}
{
    {{ discriminant_type }} _variant;
    {%- if is_struct and is_union_projected %}
    bool _hasValue;
    {%- endif %}
    {%- for variant in variants %}
    {%- for payload in variant.payloads %}
    {{ payload.type }} _{{ payload.name }};
    {%- endfor %}
    {%- endfor %}
}
