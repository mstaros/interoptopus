{%- if payload -%}
public readonly record struct {{ case_type }}({{ payload }} Value);
{%- else -%}
public readonly record struct {{ case_type }}();
{%- endif -%}
