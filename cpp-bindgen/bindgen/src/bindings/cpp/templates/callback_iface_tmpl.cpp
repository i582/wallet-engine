{%- let ffi_converter_name = typ|ffi_converter_name %}
{%- let class_name = ffi_converter_name|class_name %}
{%- let canonical_type_name = typ|canonical_name %}
{%- let trait_impl = canonical_type_name|callback_interface_name %}

{%- for (ffi_callback, method) in vtable_methods.iter() %}
 {% call macros::ffi_return_type(ffi_callback) %} {{ trait_impl}}::{{ method.name()|var_name }}({% call macros::arg_list_ffi_decl_xx(ffi_callback) %}) {
    auto obj = {{ ffi_converter_name }}::handle_map.at(uniffi_handle);

    auto make_call = [&]() {% match method.return_type() %}{% when Some(t) %}-> {{ t|type_name(ci) }}{% when None %}{% endmatch %} {
        {%- for arg in method.arguments() %}
        auto arg{{ loop.index0 }} = {{- arg|lift_fn }}({{ arg.name()|var_name }});
        {%- endfor -%}

        {%- if method.return_type().is_some() %}return {% endif -%}
         obj->{{ method.name()|var_name }}(
        {%- for arg in method.arguments() %}
        arg{{ loop.index0 }}{%- if !loop.last %}, {% else %}{% endif %}
        {%- endfor -%}
        );
    };

    {%- if method.is_async() %}
    {%- let future_result = method.foreign_future_ffi_result_struct() %}
    using CompleteCallback = void (*)(uint64_t, {{ future_result.name()|ffi_struct_name }});
    auto complete = reinterpret_cast<CompleteCallback>(uniffi_future_callback);
    {{ future_result.name()|ffi_struct_name }} result = {};
    uniffi_out_dropped_callback.handle = 0;
    uniffi_out_dropped_callback.free = reinterpret_cast<void *>(&foreign_future_drop_noop);

    {% match method.return_type() %}
    {% when Some(t) %}
    auto write_value = [&](const {{ t|type_name(ci) }} &v) {
        result.return_value = {{ t|lower_fn }}(v);
    };
    {% when None %}
    auto write_value = [](){};
    {% endmatch %}

    {% match method.throws_type() %}
    {% when Some(error) %}
        rust_call_trait_interface_with_error<{{ error|canonical_name }}>(&result.call_status, make_call, write_value, {{ error|lower_fn }});
    {% when None %}
        rust_call_trait_interface(&result.call_status, make_call, write_value);
    {% endmatch %}
    complete(uniffi_callback_data, result);
    {%- else %}
    {% match method.return_type() %}
    {% when Some(t) %}
    auto write_value = [&]({{ t|type_name(ci) }} v) {
        uniffi_out_return = {{ t|lower_fn }}(v);
    };
    {% when None %}
    auto write_value = [](){};
    {% endmatch %}

    {% match method.throws_type() %}
    {% when Some(error) %}
        rust_call_trait_interface_with_error<{{ error|canonical_name }}>(out_status, make_call, write_value, {{ error|lower_fn }});
    {% when None %}
        rust_call_trait_interface(out_status, make_call, write_value);
    {% endmatch %}
    {%- endif %}
}
{%- endfor %}

void {{ trait_impl }}::uniffi_free(uint64_t uniffi_handle) {
    {{ ffi_converter_name }}::handle_map.erase(uniffi_handle);
}

uint64_t {{ trait_impl }}::uniffi_clone(uint64_t uniffi_handle) {
    return {{ ffi_converter_name }}::handle_map.insert(
        {{ ffi_converter_name }}::handle_map.at(uniffi_handle)
    );
}

void {{ trait_impl }}::init() {
    {{ ffi_init_callback.name() }}(vtable);
}
