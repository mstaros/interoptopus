public partial class {{ class_name }}{% if dispose_body | trim %} : IDisposable{% endif %}
{
{%- for field in field_decls %}
    {{ field }}
{%- endfor %}
{%- if dispose_body | trim %}

    public void Dispose()
    {
        {{ dispose_body | indent(width = 8) }}
    }
{%- endif %}
}
