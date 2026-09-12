{%- if docs %}
{{ docs }}
{%- endif %}
{{ _fns_decorators_all }}
{% if is_value_task %}[global::System.Runtime.CompilerServices.AsyncMethodBuilder(typeof(global::System.Runtime.CompilerServices.PoolingAsyncValueTaskMethodBuilder<>))]
{% endif %}{{ visibility }} static async {% if is_value_task %}ValueTask{% else %}Task{% endif %}<{{ name }}> {{ method_name }}({% for arg in args %}{{arg.ty}} {{arg.name}}{% if arg.has_default == "true" %} = {{arg.default_value}}{% endif %}{% if not loop.last %}, {% endif %}{% endfor %})
{
    var self = new {{ name }}();
    self.__handle.Initialize(await Interop.{{ interop_name }}({% for arg in args %}{% if arg.is_ref %}ref {% endif %}{{arg.name}}{% if not loop.last %}, {% endif %}{% endfor %}));
    return self;
}
