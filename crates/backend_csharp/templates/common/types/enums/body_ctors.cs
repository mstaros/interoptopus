// Ctors
{%- for v in variants %}
{%- if v.docs %}
{{ v.docs }}
{%- endif %}
{%- if v.has_payload %}
public static {{ name }} {{ v.name }}({{ v.type }} value) => new() { _variant = {{ v.id }}, _{{ v.name }} = value{% if writes_has_value %}, _hasValue = true{% endif %} };
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
public {{ name }}({{ v.case_type }} value) { _variant = {{ v.id }}; _{{ v.name }} = value.Value;{% if writes_has_value %} _hasValue = true;{% endif %} }
{%- else %}
public {{ name }}({{ v.case_type }} value) { _variant = {{ v.id }};{% if writes_has_value %} _hasValue = true;{% endif %} }
{%- endif %}
{%- endfor %}
{%- endif %}
