{% macro rust_call(func) -%}
{%- if func.is_async() %}
    uniffi::rust_call_async(
        [&]() { return {{ func.ffi_func().name() }}(
        {%- call arg_list_lowered(func) -%}); },
        {{ func.ffi_rust_future_poll(ci) }},
        {{ func.ffi_rust_future_complete(ci) }},
        {{ func.ffi_rust_future_free(ci) }},
{%- match func.throws_type() %}
{% when Some with (e) %}
        uniffi::{{ e|ffi_error_converter_name }}::lift
{%- else %}
        nullptr
{%- endmatch %}
    )
{%- else %}
    uniffi::rust_call(
        {{ func.ffi_func().name() }},
{%- match func.throws_type() %}
{% when Some with (e) %}
        uniffi::{{ e|ffi_error_converter_name }}::lift
{%- else %}
        nullptr
{%- endmatch %}
{%- if !func.arguments().is_empty() %}, {% else %}{% endif %}
        {%- call arg_list_lowered(func) -%})
{%- endif %}
{%- endmacro %}

{% macro rust_call_with_prefix(prefix, func) -%}
{%- if func.is_async() %}
    uniffi::rust_call_async(
        [&]() { return {{ func.ffi_func().name() }}(
        {{ prefix }}
{%- if !func.arguments().is_empty() %}, {% else %}{% endif %}
        {%- call arg_list_lowered(func) -%}); },
        {{ func.ffi_rust_future_poll(ci) }},
        {{ func.ffi_rust_future_complete(ci) }},
        {{ func.ffi_rust_future_free(ci) }},
{%- match func.throws_type() %}
{% when Some with (e) %}
        uniffi::{{ e|ffi_error_converter_name }}::lift
{%- else %}
        nullptr
{%- endmatch %}
    )
{%- else %}
    uniffi::rust_call(
        {{ func.ffi_func().name() }},
{%- match func.throws_type() %}
{% when Some with (e) %}
        uniffi::{{ e|ffi_error_converter_name }}::lift,
{%- else %}
        nullptr,
{%- endmatch %}
        {{ prefix }}
{%- if !func.arguments().is_empty() %}, {% else %}{% endif %}
        {%- call arg_list_lowered(func) -%})
{%- endif %}
{%- endmacro %}

{% macro param_list(func) %}
{%- for arg in func.arguments() -%}
{{ arg|parameter(ci) }}
{%- if !loop.last -%}, {% endif -%}
{% endfor -%}
{% endmacro %}

{% macro field_name(field, field_num) %}
{%- if field.name().is_empty() -%}
v{{- field_num -}}
{%- else -%}
{{ field.name()|var_name }}
{%- endif -%}
{%- endmacro %}

{% macro arg_list_lowered(func) %}
{%- for arg in func.arguments() -%}
uniffi::{{ arg|lower_fn }}({{ arg.name()|var_name }})
{%- if !loop.last -%}, {% endif -%}
{% endfor -%}
{% endmacro %}

{%- macro arg_list_ffi_decl_xx(func) %}
    {%- for arg in func.arguments() %}
        {{- arg.type_().borrow()|ffi_type_name }} {{ arg.name()|var_name -}}{%- if !loop.last || func.has_rust_call_status_arg() -%},{%- endif -%}
    {%- endfor %}
    {%- if func.has_rust_call_status_arg() %}RustCallStatus *out_status{% endif %}
{%- endmacro -%}

{%- macro ffi_return_type(func) %}
    {%- match func.return_type() %}
    {%- when Some(return_type) %}{{ return_type|ffi_type_name }}
    {%- when None %}{{ "void" }}
    {%- endmatch %}
{%- endmacro %}

{%- macro docstring_value(maybe_docstring, indent_spaces) %}
{%- match maybe_docstring %}
{%- when Some(docstring) %}
{{ docstring|docstring(indent_spaces) }}
{%- else %}
{%- endmatch %}
{%- endmacro %}

{%- macro docstring(defn, indent_spaces) %}
{%- call docstring_value(defn.docstring(), indent_spaces) %}
{%- endmacro %}
