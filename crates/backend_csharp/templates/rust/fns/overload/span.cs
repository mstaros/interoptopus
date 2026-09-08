/// Borrows span memory for this call. The native function must not retain its pointer.
{{ _fns_decorators_all }}
{{ visibility }} static unsafe {{ rval }} {{ name }}({% for arg in args %}{{ arg.ty }} {{ arg.name }}{% if not loop.last %}, {% endif %}{% endfor %})
{
{% for pin in pins %}    fixed ({{ pin.element }}* {{ pin.pointer }} = {{ pin.name }})
{% endfor %}    {
        {% if not is_void %}return {% endif %}{{ raw_name }}({% for arg in args %}{{ arg.call }}{% if not loop.last %}, {% endif %}{% endfor %});
    }
}
