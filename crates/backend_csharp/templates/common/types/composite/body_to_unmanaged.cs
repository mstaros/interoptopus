{{ _fns_decorators_all }}
{{ _fns_decorators_internal }}
internal Unmanaged {{ to_unmanaged }}()
{
    var _unmanaged = new Unmanaged();
    {%- if to_unmanaged == "IntoUnmanaged" %}
    try
    {
        {%- for field in fields %}
        {%- if field.custom_to_unmanaged %}
        {{ field.custom_to_unmanaged | indent(width = 8) }}
        {%- else %}
        _unmanaged.{{ field.name }} = {{ field.name }}{{ field.to_unmanaged }};
        {%- endif %}
        {%- endfor %}
        return _unmanaged;
    }
    catch
    {
        _unmanaged.Free();
        throw;
    }
    {%- else %}
    {%- for field in fields %}
    {%- if field.custom_to_unmanaged %}
    {{ field.custom_to_unmanaged | indent }}
    {%- else %}
    _unmanaged.{{ field.name }} = {{ field.name }}{{ field.to_unmanaged }};
    {%- endif %}
    {%- endfor %}
    return _unmanaged;
    {%- endif %}
}
