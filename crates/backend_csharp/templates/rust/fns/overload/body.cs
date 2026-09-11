{%- if docs %}
{{ docs }}
{%- endif %}
{{ _fns_decorators_all }}
{% if is_value_task %}[global::System.Runtime.CompilerServices.AsyncMethodBuilder(typeof(global::System.Runtime.CompilerServices.PoolingAsyncValueTaskMethodBuilder{% if not is_task_void %}<>{% endif %}))]
{% endif %}{{ visibility }} static {% if is_async %}async {% endif %}{% if has_wraps and not is_async %}unsafe {% endif %}{{rval}} {{name}}({% for arg in args %}{{arg.ty}} {{arg.name}}{% if arg.has_default == "true" %} = {{arg.default_value}}{% endif %}{% if not loop.last %}, {% endif %}{% endfor %})
{
{% if is_async %}    _ct = global::Rust.Linq.ScriptScope.ResolveCancellation(_ct);
    _ct.ThrowIfCancellationRequested();
{% endif %}{{ call_body | indent(width = 4, first = true) }}
}
