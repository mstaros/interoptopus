public partial class {{ name }} : IDisposable
{
    private IntPtr _context;
    private readonly object __lifetimeGate = new();
    private int __activeCalls;
    private bool __disposeRequested;

    private {{ name }}() {}

    {% for ctor in ctors %}
    {{ ctor | indent(width = 4) }}
    {% endfor %}

    {% for method in methods %}
    {{ method | indent(width = 4) }}
    {% endfor %}

    /// <summary>Prevents new calls and releases native ownership after active calls finish.</summary>
    /// <remarks>Does not wait for active calls. Concurrent calls must still obey the Rust service's threading contract.</remarks>
    {{ _fns_decorators_all | indent }}
    public void Dispose()
    {
        IntPtr context = IntPtr.Zero;
        lock (__lifetimeGate)
        {
            if (__disposeRequested) return;
            __disposeRequested = true;
            if (__activeCalls == 0)
            {
                context = _context;
                _context = IntPtr.Zero;
            }
        }
        // Never wait for calls or run native destruction while holding the gate:
        // a native callback can dispose this service from inside an active call.
        if (context != IntPtr.Zero) Interop.{{ dtor }}(context);
    }

    internal IntPtr __AcquireCall()
    {
        lock (__lifetimeGate)
        {
            ObjectDisposedException.ThrowIf(__disposeRequested || _context == IntPtr.Zero, this);
            __activeCalls = checked(__activeCalls + 1);
            return _context;
        }
    }

    internal void __ReleaseCall()
    {
        IntPtr context = IntPtr.Zero;
        lock (__lifetimeGate)
        {
            --__activeCalls;
            if (__disposeRequested && __activeCalls == 0)
            {
                context = _context;
                _context = IntPtr.Zero;
            }
        }
        if (context != IntPtr.Zero) Interop.{{ dtor }}(context);
    }
}
