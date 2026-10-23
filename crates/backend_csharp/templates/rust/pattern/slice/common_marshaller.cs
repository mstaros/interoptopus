public partial class {{ name }}
{
    [CustomMarshaller(typeof({{ name }}), MarshalMode.ManagedToUnmanagedIn, typeof(InMarshaller))]
    public struct InMarshallerMeta { }

    public ref struct InMarshaller
    {
        private {{ name }} _managed;
        private global::System.Collections.Generic.List<Action> _releases;

        {{ _fns_decorators_all | indent(width = 8) }}
        public void FromManaged({{ name }} managed) { _managed = managed; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public Unmanaged ToUnmanaged() { return _managed.AsUnmanaged(_releases ??= new()); }

        {{ _fns_decorators_all | indent(width = 8) }}
        public void Free()
        {
            if (_releases == null) return;
            for (var i = _releases.Count - 1; i >= 0; --i) _releases[i]();
            _releases.Clear();
        }
    }

    public ref struct Marshaller
    {
        private {{ name }} _managed;
        private Unmanaged _unmanaged;
        private global::System.Collections.Generic.List<Action> _releases;

        {{ _fns_decorators_all | indent(width = 8) }}
        public Marshaller({{ name }} managed) { _managed = managed; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public Marshaller(Unmanaged unmanaged) { _unmanaged = unmanaged; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public void FromManaged({{ name }} managed) { _managed = managed; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public void FromUnmanaged(Unmanaged unmanaged) { _unmanaged = unmanaged; }

        {{ _fns_decorators_all | indent(width = 8) }}
        public Unmanaged ToUnmanaged() { return _managed.AsUnmanaged(_releases ??= new()); }

        {{ _fns_decorators_all | indent(width = 8) }}
        public {{ name }} ToManaged()
        {
            // An unchanged `ref` round-trip keeps the caller's wrapper, so a slice it owns or pins
            // stays owned or pinned; a descriptor the callee replaced becomes a borrowed view.
            if (_managed is not null && _managed._data == _unmanaged._data && _managed._len == _unmanaged._len) return _managed;
            return _unmanaged.ToManaged();
        }

        {{ _fns_decorators_all | indent(width = 8) }}
        public void Free()
        {
            if (_releases == null) return;
            for (var i = _releases.Count - 1; i >= 0; --i) _releases[i]();
            _releases.Clear();
        }
    }
}