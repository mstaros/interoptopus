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
        public {{ name }} ToManaged() { return _unmanaged.ToManaged(); }

        {{ _fns_decorators_all | indent(width = 8) }}
        public void Free()
        {
            if (_releases == null) return;
            for (var i = _releases.Count - 1; i >= 0; --i) _releases[i]();
            _releases.Clear();
        }
    }
}