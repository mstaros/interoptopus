internal class {{ trampoline_name }}
{
    private static long Id;
    private static readonly Dictionary<IntPtr, PendingCall> InFlight = new(1024);
    private sealed class PendingCall : global::System.Threading.Tasks.Sources.IValueTaskSource<{% if is_task_void %}bool{% else %}{{ task_inner_ty }}{% endif %}>
    {
        internal global::System.Threading.Tasks.Sources.ManualResetValueTaskSourceCore<{% if is_task_void %}bool{% else %}{{ task_inner_ty }}{% endif %}> Completion = new() { RunContinuationsAsynchronously = true };
        internal readonly CancellationToken Token;
        internal PendingCall(CancellationToken token) { Token = token; }
        public {% if is_task_void %}bool{% else %}{{ task_inner_ty }}{% endif %} GetResult(short token) => Completion.GetResult(token);
        public global::System.Threading.Tasks.Sources.ValueTaskSourceStatus GetStatus(short token) => Completion.GetStatus(token);
        public void OnCompleted(Action<object?> continuation, object? state, short token, global::System.Threading.Tasks.Sources.ValueTaskSourceOnCompletedFlags flags)
            => Completion.OnCompleted(continuation, state, token, flags);
    }

    private readonly AsyncCallbackCommon _delegate;
    private readonly IntPtr _callback_ptr;

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal {{ trampoline_name }}()
    {
        _delegate = Call;
        _callback_ptr = Marshal.GetFunctionPointerForDelegate(_delegate);
    }

    {{ _fns_decorators_all | indent }}
    private static unsafe void Call(IntPtr data, IntPtr csPtr)
    {
        PendingCall pending;
        lock (InFlight)
        {
            if (!InFlight.Remove(csPtr, out pending)) return;
        }

        try
        {
            // Only Ok has a payload. Never read one for cancellation, panic or an invalid tag.
            if (data == IntPtr.Zero) throw new InvalidOperationException("Rust supplied a null async result.");
            var tag = Marshal.ReadByte(data);
            if (tag == AsyncOutcomeTag.Cancelled)
            {
                pending.Completion.SetException(new OperationCanceledException("Async operation was cancelled by Rust.", pending.Token));
                return;
            }
            if (tag == AsyncOutcomeTag.Panicked)
                throw new InvalidOperationException("The Rust async operation panicked.");
            if (tag != AsyncOutcomeTag.Ok)
                throw new InvalidOperationException("Rust supplied an invalid async outcome tag.");

        {% if shape == "BareVoid" -%}
        pending.Completion.SetResult(true);
        {%- elif shape == "BareDirect" -%}
        var outcome = *({{ payload_full }}*)data;
        pending.Completion.SetResult(outcome.Value);
        {%- elif shape == "BareUnmanaged" -%}
        var outcome = *({{ payload_full }}*)data;
        var managed = outcome.Value.{{ result_to_managed }}();
        pending.Completion.SetResult(managed);
        {%- elif shape == "ResultDirect" -%}
        var outcome = *({{ payload_full }}*)data;
        var managed = outcome.Value;
        if (managed.IsOk) { pending.Completion.SetResult({% if is_task_void %}true{% else %}managed.AsOk(){% endif %}); }
        else { pending.Completion.SetException(managed.ExceptionForVariant()); }
        {%- elif shape == "ResultUnmanaged" -%}
        var outcome = *({{ payload_full }}*)data;
        var managed = outcome.Value.{{ result_to_managed }}();
        if (managed.IsOk) { pending.Completion.SetResult({% if is_task_void %}true{% else %}managed.AsOk(){% endif %}); }
        else { pending.Completion.SetException(managed.ExceptionForVariant()); }
        {%- endif %}
        }
        catch (Exception error)
        {
            // Managed conversion errors must fault the await, never unwind across reverse P/Invoke.
            pending.Completion.SetException(error);
        }
    }

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal (AsyncCallbackCommonNative, ValueTask<{% if is_task_void %}bool{% else %}{{ task_inner_ty }}{% endif %}>) NewCall(CancellationToken token = default)
    {
        var pending = new PendingCall(token);
        IntPtr id;
        lock (InFlight)
        {
            do { id = unchecked((IntPtr)(nint)Interlocked.Increment(ref Id)); }
            while (id == IntPtr.Zero || !InFlight.TryAdd(id, pending));
        }
        var callback = new AsyncCallbackCommonNative { _ptr = _callback_ptr, _ts = id };
        return (callback, new ValueTask<{% if is_task_void %}bool{% else %}{{ task_inner_ty }}{% endif %}>(pending, pending.Completion.Version));
    }

    // Only used when starting the native call failed before a task handle was returned.
    internal void Abandon(AsyncCallbackCommonNative callback)
    {
        lock (InFlight) { InFlight.Remove(callback._ts); }
    }
}
