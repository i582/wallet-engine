{%- let obj = ci.get_object_definition(name).unwrap() %}
{%- let (interface_name, impl_class_name) = obj|object_names %}

{%- let is_error = ci.is_name_used_as_error(name) %}
{%- if is_error %}
{{ type_name }} {{ typ|ffi_error_converter_name}}::lift(RustBuffer buf) {
    auto stream = RustStream(&buf);
    auto val = {{ ffi_converter_name }}::read(stream);
    rustbuffer_free(buf);

    return val;
}
{% endif %}

{{ type_name }} {{ ffi_converter_name }}::lift(uint64_t ptr) {
    return {{ type_name }}(new {{ impl_class_name }}(ptr));
}

uint64_t {{ ffi_converter_name }}::lower(const {{ type_name }} &obj) {
    {%- if obj.has_callback_interface() %}
    return handle_map.insert(obj);
    {%- else %}
    return reinterpret_cast<{{ impl_class_name}}*>(obj.get())->_uniffi_internal_clone_pointer();
    {%- endif %}
}

{{ type_name }} {{ ffi_converter_name }}::read(RustStream &stream) {
    uint64_t ptr;
    stream >> ptr;

    return {{ ffi_converter_name}}::lift(ptr);
}

void {{ ffi_converter_name }}::write(RustStream &stream, const {{ type_name }} &obj) {
    stream << {{ ffi_converter_name }}::lower(obj);
}

uint64_t {{ ffi_converter_name }}::allocation_size(const {{ type_name }} &) {
    return 8;
}
