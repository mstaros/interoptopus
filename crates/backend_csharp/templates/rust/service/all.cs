public partial class {{ name }} : IDisposable
{
    private IntPtr _context;

    private {{ name }}() {}

    {% for ctor in ctors %}
    {{ ctor | indent(width = 4) }}
    {% endfor %}

    {% for method in methods %}
    {{ method | indent(width = 4) }}
    {% endfor %}

    {{ _fns_decorators_all | indent }}
    public void Dispose()
    {
        var context = System.Threading.Interlocked.Exchange(ref _context, IntPtr.Zero);
        if (context != IntPtr.Zero) Interop.{{ dtor }}(context);
    }

    internal IntPtr Context
    {
        get
        {
            var context = _context;
            ObjectDisposedException.ThrowIf(context == IntPtr.Zero, this);
            return context;
        }
    }
}
