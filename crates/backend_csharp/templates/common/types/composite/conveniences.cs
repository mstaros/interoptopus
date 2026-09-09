{% if is_value %}
[global::System.Diagnostics.CodeAnalysis.SetsRequiredMembers]
public {{ name }}({% for field in fields %}{{ field.ty }} {{ field.name }}{% if not loop.last %}, {% endif %}{% endfor %})
{
    this = default;
    {%- for field in fields %}
    this.{{ field.name }} = {{ field.name }};
    {%- endfor %}
}
{% if deconstruct %}
public void Deconstruct({% for field in fields %}out {{ field.ty }} value{{ loop.index }}{% if not loop.last %}, {% endif %}{% endfor %})
{
    {%- for field in fields %}
    value{{ loop.index }} = this.{{ field.name }};
    {%- endfor %}
}
{% endif %}
{% endif %}
{% if tuple_type %}
public static implicit operator {{ tuple_type }}({{ name }} value)
    => ({% for field in fields %}value.{{ field.name }}{% if not loop.last %}, {% endif %}{% endfor %});

public static implicit operator {{ name }}({{ tuple_type }} value)
    => new {{ name }} { {% for field in fields %}{{ field.name }} = value.Item{{ loop.index }}{% if not loop.last %}, {% endif %}{% endfor %} };

[CustomMarshaller(typeof({{ tuple_type }}), MarshalMode.Default, typeof(TupleMarshaller))]
public struct TupleMarshallerMeta { }

public ref struct TupleMarshaller
{
    private {{ name }} _managed;
    private Unmanaged _unmanaged;
    public void FromManaged({{ tuple_type }} value) { _managed = value; }
    public void FromUnmanaged(Unmanaged value) { _unmanaged = value; }
    public Unmanaged ToUnmanaged() => _managed.ToUnmanaged();
    public {{ tuple_type }} ToManaged() => _unmanaged.ToManaged();
    public void Free() { }
}
{% endif %}
