// Ctors
{%- for v in variants %}
{%- if v.docs %}
{{ v.docs }}
{%- endif %}
{%- if v.has_payload %}
public static {{ name }} {{ v.name }}({% for payload in v.payloads %}{{ payload.type }} value{% if not loop.last %}, {% endif %}{% endfor %}) => new() { _variant = {{ v.id }}{% for payload in v.payloads %}, _{{ payload.name }} = value{% endfor %}{% if writes_has_value %}, _hasValue = true{% endif %} };
{%- else %}
public static {{ name }} {{ v.name }} => new() { _variant = {{ v.id }}{% if writes_has_value %}, _hasValue = true{% endif %} };
{%- endif %}
{%- endfor %}

// Checks
{%- for v in variants %}
public bool Is{{ v.name }} => {% if writes_has_value %}_hasValue && {% endif %}_variant == {{ v.id }};
{%- endfor %}

// Conversions
{%- for v in variants %}
{%- if v.has_payload %}
public {{ v.type }} As{{ v.name }}() { if ({% if writes_has_value %}!_hasValue || {% endif %}_variant != {{ v.id }}) { throw ExceptionForVariant(); } else { return _{{ v.name }}; } }
{%- else %}
public void As{{ v.name }}() { if ({% if writes_has_value %}!_hasValue || {% endif %}_variant != {{ v.id }}) throw ExceptionForVariant(); }
{%- endif %}
{%- endfor %}
{%- if is_union_projected %}

// Case constructors — the union creation members the compiler reads case types from.
{%- for v in variants %}
{%- if v.has_payload %}
public {{ name }}({{ v.case_type }} value) { _variant = {{ v.id }};{% for payload in v.payloads %} _{{ payload.name }} = value.Value;{% endfor %}{% if writes_has_value %} _hasValue = true;{% endif %} }
{%- else %}
public {{ name }}({{ v.case_type }} value) { _variant = {{ v.id }};{% if writes_has_value %} _hasValue = true;{% endif %} }
{%- endif %}
{%- endfor %}
{%- endif %}
