public partial class {{ name }} : IDisposable
{
    [StructLayout(LayoutKind.Sequential)]
    internal struct Unmanaged
    {
        internal IntPtr _handle;

        internal void Free()
        {
            var handle = _handle;
            _handle = IntPtr.Zero;
            if (handle != IntPtr.Zero) GCHandle.FromIntPtr(handle).Free();
        }

        {{ _fns_decorators_all | indent(width = 8) }}
        {{ _fns_decorators_internal | indent(width = 8) }}
        internal {{ name }} IntoManaged()
        {
            var h = GCHandle.FromIntPtr(_handle);
            var obj = ({{ name }})h.Target!;
            h.Free();
            return obj;
        }

        {{ _fns_decorators_all | indent(width = 8) }}
        {{ _fns_decorators_internal | indent(width = 8) }}
        internal {{ name }} AsManaged()
        {
            return ({{ name }})GCHandle.FromIntPtr(_handle).Target!;
        }
    }

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal Unmanaged IntoUnmanaged(global::System.Collections.Generic.List<Action> releases = null)
    {
        var h = GCHandle.Alloc(this);
        return new Unmanaged { _handle = GCHandle.ToIntPtr(h) };
    }

    {{ _fns_decorators_all | indent }}
    {{ _fns_decorators_internal | indent }}
    internal Unmanaged AsUnmanaged(global::System.Collections.Generic.List<Action> releases)
    {
        var h = GCHandle.Alloc(this);
        try { releases.Add(() => h.Free()); }
        catch { h.Free(); throw; }
        return new Unmanaged { _handle = GCHandle.ToIntPtr(h) };
    }

    internal Unmanaged AsUnmanaged()
    {
        var h = GCHandle.Alloc(this);
        return new Unmanaged { _handle = GCHandle.ToIntPtr(h) };
    }

    public void Dispose() { }
}
