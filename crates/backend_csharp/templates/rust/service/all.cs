/// <remarks>
/// Native ownership is released by SafeHandle if disposal is omitted. Native destruction must be
/// safe on the finalizer thread or the thread releasing the last call, and must not panic or throw.
{% if has_mutable_calls -%}
/// Interlocked guards reject overlapping calls involving mutable access with InvalidOperationException,
/// including callback reentry. Shared calls may overlap subject to the Rust threading contract.
/// Guards cover call duration only. Raw pointer calls and returned borrows require caller coordination.
{% endif -%}
/// </remarks>
public partial class {{ name }} : IDisposable
{
    // Allocate the owner before a native constructor can return a resource.
    private readonly __ServiceHandle __handle = new();
    private readonly object __lifetimeGate = new();
    private bool __disposeRequested;{% if has_mutable_calls %}
    // -1 is an exclusive borrow; nonnegative values count shared borrows.
    private int __borrowState;{% endif %}

    private sealed class __ServiceHandle : SafeHandle
    {
        internal __ServiceHandle() : base(IntPtr.Zero, true) {}
        public override bool IsInvalid => handle == IntPtr.Zero;
        internal void Initialize(IntPtr context) => SetHandle(context);

        protected override bool ReleaseHandle()
        {
            try
            {
                Interop.{{ dtor }}(handle);
                return true;
            }
            catch
            {
                // Cleanup must not let managed interop errors escape the finalizer.
                // A native panic cannot be contained here; the destructor must not panic.
                return false;
            }
        }
    }

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
        lock (__lifetimeGate)
        {
            if (__disposeRequested) return;
            __disposeRequested = true;
        }
        // Never wait for calls or run native destruction while holding the gate:
        // a native callback can dispose this service from inside an active call.
        __handle.Dispose();
    }

    internal IntPtr __AcquireCall({% if has_mutable_calls %}bool exclusive = false{% endif %})
    {
        bool acquired = false;
        try
        {
            lock (__lifetimeGate)
            {
                // SafeHandle may accept more references after Dispose while calls still hold it.
                ObjectDisposedException.ThrowIf(__disposeRequested || __handle.IsInvalid, this);
                __handle.DangerousAddRef(ref acquired);
            }
{% if has_mutable_calls %}            if (exclusive)
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
{% endif %}            return __handle.DangerousGetHandle();
        }
        catch
        {
            // Failed access never owns a borrow, but must release its lifetime reservation.
            if (acquired) __handle.DangerousRelease();
            throw;
        }
    }

    internal void __ReleaseCall({% if has_mutable_calls %}bool exclusive = false{% endif %})
    {
{% if has_mutable_calls %}        if (exclusive) Interlocked.Exchange(ref __borrowState, 0);
        else Interlocked.Decrement(ref __borrowState);
{% endif %}        __handle.DangerousRelease();
    }
}
