[StructLayout(LayoutKind.Sequential)]
internal unsafe struct Unmanaged{{ variant }}
{
    internal {{ discriminant_type }} _variant;
    {%- for payload in payloads %}
    internal {{ payload.unmanaged_name }} {{ payload.field }};
    {%- endfor %}
}
