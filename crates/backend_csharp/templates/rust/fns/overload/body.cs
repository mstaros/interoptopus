{%- if docs %}
{{ docs }}
{%- endif %}
{{ _fns_decorators_all }}
{% if is_value_task %}[global::System.Runtime.CompilerServices.AsyncMethodBuilder(typeof(global::System.Runtime.CompilerServices.PoolingAsyncValueTaskMethodBuilder{% if not is_task_void %}<>{% endif %}))]
{% endif %}{{ visibility }} static {% if is_async %}async {% endif %}{% if has_wraps and not is_async %}unsafe {% endif %}{{rval}} {{name}}({% for arg in args %}{{arg.ty}} {{arg.name}}{% if arg.has_default == "true" %} = {{arg.default_value}}{% endif %}{% if not loop.last %}, {% endif %}{% endfor %})
{
{% if is_async %}    _ct = global::Rust.Linq.ScriptScope.ResolveCancellation(_ct);
    _ct.ThrowIfCancellationRequested();
{% endif %}{% for arg in args %}{% if arg.is_wrap == "true" %}    using var {{arg.name}}_wrapped = new {{arg.wrapper_type}}({{arg.name}});
{% endif %}{% endfor %}{% if is_async %}    var (_cb, _cs) = Interop.{{ trampoline_field }}.NewCall(_ct);
    TaskHandle _th = default;
    CancellationTokenRegistration _cr = default;
    bool _started = false;
    try
    {
        _th = {{ name }}({% for arg in native_args %}{{arg.name}}, {% endfor %}_cb);
        _started = true;
        try { _cr = _ct.Register(() => { unsafe { _th.Abort(); } }); }
        catch
        {
            unsafe { _th.Abort(); }
            // Native code still owns the callback. Wait for acknowledgement before releasing arguments.
            try { await _cs.ConfigureAwait(false); } catch { }
            throw;
        }
        {% if is_task_void %}await _cs.ConfigureAwait(false);{% else %}return await _cs.ConfigureAwait(false);{% endif %}
    }
    finally
    {
        if (!_started) Interop.{{ trampoline_field }}.Abandon(_cb);
        try { _cr.Dispose(); }
        finally { unsafe { _th.Dispose(); } }
    }
{% else %}    {% if not is_void %}return {% endif %}{{name}}({% for arg in native_args %}{{arg.name}}{% if not loop.last %}, {% endif %}{% endfor %});
{% endif %}}
