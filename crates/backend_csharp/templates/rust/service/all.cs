{% if has_mutable_calls -%}
/// <remarks>
/// Interlocked guards reject overlapping calls involving mutable access with InvalidOperationException,
/// including callback reentry. Shared calls may overlap subject to the Rust threading contract.
/// Guards cover call duration only. Raw pointer calls and returned borrows require caller coordination.
/// </remarks>
{% endif -%}
public partial class {{ name }} : IDisposable
{
    private IntPtr _context;
    private readonly object __lifetimeGate = new();
    private int __activeCalls;
    private bool __disposeRequested;{% if has_mutable_calls %}
    // -1 is an exclusive borrow; nonnegative values count shared borrows.
    private int __borrowState;{% endif %}

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

    internal IntPtr __AcquireCall({% if has_mutable_calls %}bool exclusive = false{% endif %})
    {
{% if has_mutable_calls %}        IntPtr context;
{% endif %}        lock (__lifetimeGate)
        {
            ObjectDisposedException.ThrowIf(__disposeRequested || _context == IntPtr.Zero, this);
            __activeCalls = checked(__activeCalls + 1);
{% if has_mutable_calls %}            context = _context;
{% else %}            return _context;
{% endif %}        }
{% if has_mutable_calls %}        try
        {
            if (exclusive)
            {
                if (Interlocked.CompareExchange(ref __borrowState, -1, 0) != 0)
                    throw new InvalidOperationException("Cannot borrow {{ name }} exclusively while another call is active.");
            }
            else
            {
                int readers;
                do
                {
                    readers = Volatile.Read(ref __borrowState);
                    if (readers < 0)
                        throw new InvalidOperationException("Cannot borrow {{ name }} while a mutable call is active.");
                }
                // Retry only a racing state change; never wait for a borrow to finish.
                while (Interlocked.CompareExchange(ref __borrowState, checked(readers + 1), readers) != readers);
            }
            return context;
        }
        catch
        {
            // Failed access never owns a borrow, but must release its lifetime reservation.
            __ReleaseLifetime();
            throw;
        }
{% endif %}    }

{% if has_mutable_calls %}    internal void __ReleaseCall(bool exclusive = false)
    {
        if (exclusive) Interlocked.Exchange(ref __borrowState, 0);
        else Interlocked.Decrement(ref __borrowState);
        __ReleaseLifetime();
    }

    private void __ReleaseLifetime()
{% else %}    internal void __ReleaseCall()
{% endif %}    {
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
