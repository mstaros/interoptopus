{%- if docs %}
{{ docs }}
{%- endif %}
{{ _fns_decorators_all }}
{{ visibility }} {{ rval }} {{ method_name }}({% for arg in args %}{{arg.ty}} {{arg.name}}{% if not loop.last %}, {% endif %}{% endfor %})
{
{% if self_arg == "_context" %}    __AcquireCall({% if exclusive %}true{% endif %});
    try
    {
    {% endif %}    {% if not is_void %}return {% endif %}Interop.{{ interop_name }}({% if self_arg == "_context" %}__handle.DangerousGetHandle(){% else %}{{ self_arg }}{% endif %}{% for arg in args %}, {% if arg.is_ref %}ref {% endif %}{{arg.name}}{% endfor %}){% if as_ok %}.AsOk(){% endif %};
{% if self_arg == "_context" %}    }
    finally
    {
        __ReleaseCall({% if exclusive %}true{% endif %});
    }
{% endif %}}