use std::fmt::Write as _;

use crate::{
    compound_map::CompoundTypeRef,
    enum_map::FlatEnum,
    model::BindingsModel,
    optional_map::OptionalType,
    record_map::{RecordField, RecordType},
    sequence_map::SequenceType,
    tagged_enum::{TaggedEnumField, TaggedEnumType, TaggedEnumVariant},
    template,
    type_map::BuiltinType,
    type_registry::NestedWireSize,
};

const BASE: &str = include_str!("../../templates/codecs/base.c.tmpl");
const U8_CODEC: &str = include_str!("../../templates/codecs/u8.c.tmpl");
const I8_CODEC: &str = include_str!("../../templates/codecs/i8.c.tmpl");
const U16_CODEC: &str = include_str!("../../templates/codecs/u16.c.tmpl");
const I16_CODEC: &str = include_str!("../../templates/codecs/i16.c.tmpl");
const U32_WIRE_CODEC: &str = include_str!("../../templates/codecs/u32.c.tmpl");
const I32_CODEC: &str = include_str!("../../templates/codecs/i32.c.tmpl");
const U64_CODEC: &str = include_str!("../../templates/codecs/u64.c.tmpl");
const I64_CODEC: &str = include_str!("../../templates/codecs/i64.c.tmpl");
const BOOLEAN_CODEC: &str = include_str!("../../templates/codecs/bool.c.tmpl");
const RUSTBUFFER_RUNTIME: &str = include_str!("../../templates/codecs/rustbuffer.c.tmpl");
const STRING_CODEC: &str = include_str!("../../templates/codecs/string.c.tmpl");
const BYTES_CODEC: &str = include_str!("../../templates/codecs/bytes.c.tmpl");
const FLAT_ENUM_CODEC: &str = include_str!("../../templates/codecs/flat_enum.c.tmpl");
const CUSTOM_STRING_CODEC: &str = include_str!("../../templates/codecs/custom_string.c.tmpl");
const OPTIONAL_CODEC: &str = include_str!("../../templates/codecs/optional.c.tmpl");
const OPTIONAL_ARENA_CODEC: &str = include_str!("../../templates/codecs/optional_arena.c.tmpl");
const OPTIONAL_MEASURE_DYNAMIC: &str =
    include_str!("../../templates/codecs/optional_measure_dynamic.c.tmpl");
const ARENA_RUNTIME: &str = include_str!("../../templates/codecs/arena.c.tmpl");
const SEQUENCE_CODEC: &str = include_str!("../../templates/codecs/sequence.c.tmpl");
const SEQUENCE_FIXED_MEASURE: &str =
    include_str!("../../templates/codecs/sequence_measure_fixed.c.tmpl");
const SEQUENCE_LENGTH_PREFIXED_VIEW_MEASURE: &str =
    include_str!("../../templates/codecs/sequence_measure_length_prefixed_view.c.tmpl");
const SEQUENCE_DYNAMIC_MEASURE: &str =
    include_str!("../../templates/codecs/sequence_measure_dynamic.c.tmpl");
const SEQUENCE_MINIMUM_REMAINING_CHECK: &str =
    include_str!("../../templates/codecs/sequence_minimum_remaining_check.c.tmpl");
const SEQUENCE_READ_ITEM: &str = include_str!("../../templates/codecs/sequence_read_item.c.tmpl");
const SEQUENCE_READ_ARENA_ITEM: &str =
    include_str!("../../templates/codecs/sequence_read_arena_item.c.tmpl");
const RECORD_CODEC: &str = include_str!("../../templates/codecs/record.c.tmpl");
const EMPTY_RECORD_CODEC: &str = include_str!("../../templates/codecs/empty_record.c.tmpl");
const RECORD_MEASURE_FIXED_FIELD: &str =
    include_str!("../../templates/codecs/record_measure_fixed_field.c.tmpl");
const RECORD_MEASURE_LENGTH_PREFIXED_VIEW_FIELD: &str =
    include_str!("../../templates/codecs/record_measure_length_prefixed_view_field.c.tmpl");
const RECORD_MEASURE_DYNAMIC_FIELD: &str =
    include_str!("../../templates/codecs/record_measure_dynamic_field.c.tmpl");
const RECORD_WRITE_FIELD: &str = include_str!("../../templates/codecs/record_write_field.c.tmpl");
const RECORD_READ_FIELD: &str = include_str!("../../templates/codecs/record_read_field.c.tmpl");
const RECORD_READ_ARENA_FIELD: &str =
    include_str!("../../templates/codecs/record_read_arena_field.c.tmpl");
const ERROR_CODEC: &str = include_str!("../../templates/codecs/error.c.tmpl");
const ERROR_ARENA_CODEC: &str = include_str!("../../templates/codecs/error_arena.c.tmpl");
const ERROR_MEASURE_CASE: &str = include_str!("../../templates/codecs/error_measure_case.c.tmpl");
const ERROR_MEASURE_FIXED_FIELD: &str =
    include_str!("../../templates/codecs/error_measure_fixed_field.c.tmpl");
const ERROR_MEASURE_LENGTH_PREFIXED_VIEW_FIELD: &str =
    include_str!("../../templates/codecs/error_measure_length_prefixed_view_field.c.tmpl");
const ERROR_MEASURE_DYNAMIC_FIELD: &str =
    include_str!("../../templates/codecs/error_measure_dynamic_field.c.tmpl");
const ERROR_WRITE_CASE: &str = include_str!("../../templates/codecs/error_write_case.c.tmpl");
const ERROR_WRITE_FIELD: &str = include_str!("../../templates/codecs/error_write_field.c.tmpl");
const ERROR_READ_CASE: &str = include_str!("../../templates/codecs/error_read_case.c.tmpl");
const ERROR_READ_FIELD: &str = include_str!("../../templates/codecs/error_read_field.c.tmpl");
const ERROR_READ_FIELD_WITH_ROLLBACK: &str =
    include_str!("../../templates/codecs/error_read_field_with_rollback.c.tmpl");
const ERROR_READ_ARENA_FIELD: &str =
    include_str!("../../templates/codecs/error_read_arena_field.c.tmpl");

pub(super) fn render(model: &BindingsModel) -> String {
    let mut output = if model.has_wire_types() {
        String::from(BASE)
    } else {
        String::new()
    };

    if model.needs_u8_wire_codec() {
        output.push_str(U8_CODEC);
    }
    if model.has_builtin_type(BuiltinType::Int8) {
        output.push_str(I8_CODEC);
    }
    if model.has_builtin_type(BuiltinType::UInt16) || model.has_builtin_type(BuiltinType::Int16) {
        output.push_str(U16_CODEC);
    }
    if model.has_builtin_type(BuiltinType::Int16) {
        output.push_str(I16_CODEC);
    }
    if model.has_builtin_type(BuiltinType::UInt32)
        || model.has_builtin_type(BuiltinType::Int32)
        || model.has_builtin_type(BuiltinType::String)
        || model.has_builtin_type(BuiltinType::Bytes)
        || model.needs_i32_wire_codec()
    {
        output.push_str(U32_WIRE_CODEC);
    }
    if model.needs_i32_wire_codec() {
        output.push_str(I32_CODEC);
    }
    if model.has_builtin_type(BuiltinType::UInt64) || model.has_builtin_type(BuiltinType::Int64) {
        output.push_str(U64_CODEC);
    }
    if model.has_builtin_type(BuiltinType::Int64) {
        output.push_str(I64_CODEC);
    }
    if model.has_builtin_type(BuiltinType::Boolean) {
        output.push_str(BOOLEAN_CODEC);
    }
    if model.needs_rustbuffer_runtime() {
        output.push_str(&rustbuffer_runtime(
            model.private_ffi().rustbuffer_alloc(),
            model.private_ffi().rustbuffer_free(),
        ));
    }
    if model.needs_output_arena() {
        output.push_str(ARENA_RUNTIME);
    }
    if model.has_builtin_type(BuiltinType::String) {
        output.push_str(STRING_CODEC);
    }
    if model.has_builtin_type(BuiltinType::Bytes) {
        output.push_str(BYTES_CODEC);
    }
    for enum_ in model.flat_enums() {
        output.push_str(&render_flat_enum(enum_));
    }
    for custom in model.custom_types() {
        output.push_str(&template::render(
            CUSTOM_STRING_CODEC,
            &[
                ("FUNCTION_NAME", custom.function_name()),
                ("C_NAME", custom.c_name()),
                ("BUILTIN_FUNCTION_NAME", custom.builtin_function_name()),
                ("BUILTIN_C_NAME", custom.builtin_c_name()),
            ],
        ));
    }
    for enum_ in model.fielded_enum_types() {
        output.push_str(&render_tagged_enum(enum_));
    }
    for compound in model.compound_types() {
        output.push_str(&match compound {
            CompoundTypeRef::Optional(optional) => render_optional(optional),
            CompoundTypeRef::Sequence(sequence) => render_sequence(sequence),
            CompoundTypeRef::Record(record) => render_record(record),
        });
    }
    for error in model.error_types() {
        output.push_str(&render_tagged_enum(error));
    }

    output
}

fn rustbuffer_runtime(alloc_symbol: &str, free_symbol: &str) -> String {
    template::render(
        RUSTBUFFER_RUNTIME,
        &[("ALLOC_SYMBOL", alloc_symbol), ("FREE_SYMBOL", free_symbol)],
    )
}

fn render_flat_enum(enum_: &FlatEnum) -> String {
    let mut write_cases = String::new();
    let mut read_cases = String::new();
    for variant in enum_.variants() {
        let _ = writeln!(
            write_cases,
            "        case {}:\n            wire_tag = INT32_C({});\n            break;",
            variant.c_constant(),
            variant.wire_tag(),
        );
        let _ = writeln!(
            read_cases,
            "        case INT32_C({}):\n            *out_value = {};\n            return WALLET_ENGINE_ABI_STATUS_OK;",
            variant.wire_tag(),
            variant.c_constant(),
        );
    }

    template::render(
        FLAT_ENUM_CODEC,
        &[
            ("FUNCTION_NAME", enum_.function_name()),
            ("C_NAME", enum_.c_name()),
            ("WRITE_CASES", &write_cases),
            ("READ_CASES", &read_cases),
        ],
    )
}

fn render_optional(optional: &OptionalType) -> String {
    let measure_some = match optional.inner_wire_size() {
        NestedWireSize::Fixed(inner_size) => {
            format!("        wire_size += {inner_size}u;\n")
        }
        NestedWireSize::LengthPrefixedView => String::from(
            r"        if ((value.value.len != 0u && value.value.data == NULL)
            || value.value.len > (size_t)INT32_MAX
            || value.value.len > SIZE_MAX - 5u) {
            return WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT;
        }
        wire_size = value.value.len + 5u;
",
        ),
        NestedWireSize::Dynamic => template::render(
            OPTIONAL_MEASURE_DYNAMIC,
            &[("INNER_FUNCTION_NAME", optional.inner_function_name())],
        ),
    };

    template::render(
        if optional.inner_read_needs_arena() {
            OPTIONAL_ARENA_CODEC
        } else {
            OPTIONAL_CODEC
        },
        &[
            ("FUNCTION_NAME", optional.function_name()),
            ("C_NAME", optional.c_name()),
            ("INNER_FUNCTION_NAME", optional.inner_function_name()),
            ("MEASURE_SOME", &measure_some),
        ],
    )
}

fn render_sequence(sequence: &SequenceType) -> String {
    let measure_items = match sequence.inner_wire_size() {
        NestedWireSize::Fixed(inner_size) => {
            let inner_size = inner_size.to_string();
            template::render(SEQUENCE_FIXED_MEASURE, &[("INNER_WIRE_SIZE", &inner_size)])
        }
        NestedWireSize::LengthPrefixedView => String::from(SEQUENCE_LENGTH_PREFIXED_VIEW_MEASURE),
        NestedWireSize::Dynamic => template::render(
            SEQUENCE_DYNAMIC_MEASURE,
            &[("INNER_FUNCTION_NAME", sequence.inner_function_name())],
        ),
    };
    let minimum_remaining_check = if sequence.inner_minimum_wire_size() == 0 {
        String::new()
    } else {
        template::render(
            SEQUENCE_MINIMUM_REMAINING_CHECK,
            &[(
                "MINIMUM_INNER_WIRE_SIZE",
                &sequence.inner_minimum_wire_size().to_string(),
            )],
        )
    };
    let read_item = template::render(
        if sequence.inner_read_needs_arena() {
            SEQUENCE_READ_ARENA_ITEM
        } else {
            SEQUENCE_READ_ITEM
        },
        &[("INNER_FUNCTION_NAME", sequence.inner_function_name())],
    );

    template::render(
        SEQUENCE_CODEC,
        &[
            ("FUNCTION_NAME", sequence.function_name()),
            ("C_NAME", sequence.c_name()),
            ("INNER_C_NAME", sequence.inner_c_name()),
            ("INNER_FUNCTION_NAME", sequence.inner_function_name()),
            ("MINIMUM_REMAINING_CHECK", &minimum_remaining_check),
            ("MEASURE_ITEMS", &measure_items),
            ("READ_ITEM", &read_item),
        ],
    )
}

fn render_record(record: &RecordType) -> String {
    let template_source = if record.fields().is_empty() {
        EMPTY_RECORD_CODEC
    } else {
        RECORD_CODEC
    };
    let mut measure_fields = String::new();
    let mut write_fields = String::new();
    let mut read_fields = String::new();
    for field in record.fields() {
        measure_fields.push_str(&render_record_measure_field(field));
        write_fields.push_str(&template::render(
            RECORD_WRITE_FIELD,
            &[
                ("FIELD_CODEC_NAME", field.codec_name()),
                ("FIELD_C_NAME", field.c_name()),
            ],
        ));
        let read_template = if field.read_needs_arena() {
            RECORD_READ_ARENA_FIELD
        } else {
            RECORD_READ_FIELD
        };
        read_fields.push_str(&template::render(
            read_template,
            &[
                ("FIELD_CODEC_NAME", field.codec_name()),
                ("FIELD_C_NAME", field.c_name()),
            ],
        ));
    }

    let mut replacements = vec![
        ("FUNCTION_NAME", record.function_name()),
        ("C_NAME", record.c_name()),
    ];
    if !record.fields().is_empty() {
        replacements.extend([
            ("MEASURE_FIELDS", measure_fields.as_str()),
            ("WRITE_FIELDS", write_fields.as_str()),
            ("READ_FIELDS", read_fields.as_str()),
        ]);
    }
    template::render(template_source, &replacements)
}

fn render_record_measure_field(field: &RecordField) -> String {
    match field.nested_wire_size() {
        NestedWireSize::Fixed(size) => {
            let size = size.to_string();
            template::render(RECORD_MEASURE_FIXED_FIELD, &[("FIELD_WIRE_SIZE", &size)])
        }
        NestedWireSize::LengthPrefixedView => template::render(
            RECORD_MEASURE_LENGTH_PREFIXED_VIEW_FIELD,
            &[("FIELD_C_NAME", field.c_name())],
        ),
        NestedWireSize::Dynamic => template::render(
            RECORD_MEASURE_DYNAMIC_FIELD,
            &[
                ("FIELD_CODEC_NAME", field.codec_name()),
                ("FIELD_C_NAME", field.c_name()),
            ],
        ),
    }
}

fn render_tagged_enum(enum_: &TaggedEnumType) -> String {
    let mut measure_cases = String::new();
    let mut write_cases = String::new();
    let mut read_cases = String::new();
    for variant in enum_.variants() {
        measure_cases.push_str(&render_tagged_enum_measure_case(variant));
        write_cases.push_str(&render_tagged_enum_write_case(variant));
        read_cases.push_str(&render_tagged_enum_read_case(enum_, variant));
    }
    let template_source = if enum_.read_needs_arena() {
        ERROR_ARENA_CODEC
    } else {
        ERROR_CODEC
    };

    template::render(
        template_source,
        &[
            ("FUNCTION_NAME", enum_.function_name()),
            ("C_NAME", enum_.c_name()),
            ("MEASURE_CASES", &measure_cases),
            ("WRITE_CASES", &write_cases),
            ("READ_CASES", &read_cases),
        ],
    )
}

fn render_tagged_enum_measure_case(variant: &TaggedEnumVariant) -> String {
    let mut fields = String::new();
    for field in variant.fields() {
        fields.push_str(&render_tagged_enum_measure_field(variant, field));
    }
    template::render(
        ERROR_MEASURE_CASE,
        &[
            ("TAG_CONSTANT", variant.c_constant()),
            ("MEASURE_FIELDS", &fields),
        ],
    )
}

fn render_tagged_enum_measure_field(
    variant: &TaggedEnumVariant,
    field: &TaggedEnumField,
) -> String {
    match field.nested_wire_size() {
        NestedWireSize::Fixed(size) => {
            let size = size.to_string();
            template::render(ERROR_MEASURE_FIXED_FIELD, &[("FIELD_WIRE_SIZE", &size)])
        }
        NestedWireSize::LengthPrefixedView => template::render(
            ERROR_MEASURE_LENGTH_PREFIXED_VIEW_FIELD,
            &[
                ("PAYLOAD_MEMBER_NAME", variant.payload_member_name()),
                ("FIELD_C_NAME", field.c_name()),
            ],
        ),
        NestedWireSize::Dynamic => template::render(
            ERROR_MEASURE_DYNAMIC_FIELD,
            &[
                ("PAYLOAD_MEMBER_NAME", variant.payload_member_name()),
                ("FIELD_C_NAME", field.c_name()),
                ("FIELD_CODEC_NAME", field.codec_name()),
            ],
        ),
    }
}

fn render_tagged_enum_write_case(variant: &TaggedEnumVariant) -> String {
    let mut fields = String::new();
    for field in variant.fields() {
        fields.push_str(&template::render(
            ERROR_WRITE_FIELD,
            &[
                ("PAYLOAD_MEMBER_NAME", variant.payload_member_name()),
                ("FIELD_C_NAME", field.c_name()),
                ("FIELD_CODEC_NAME", field.codec_name()),
            ],
        ));
    }
    let wire_tag = variant.wire_tag().to_string();
    template::render(
        ERROR_WRITE_CASE,
        &[
            ("TAG_CONSTANT", variant.c_constant()),
            ("WIRE_TAG", &wire_tag),
            ("WRITE_FIELDS", &fields),
        ],
    )
}

fn render_tagged_enum_read_case(enum_: &TaggedEnumType, variant: &TaggedEnumVariant) -> String {
    let mut fields = String::new();
    for field in variant.fields() {
        let read_template = if field.read_needs_arena() {
            ERROR_READ_ARENA_FIELD
        } else if enum_.read_needs_arena() {
            ERROR_READ_FIELD_WITH_ROLLBACK
        } else {
            ERROR_READ_FIELD
        };
        fields.push_str(&template::render(
            read_template,
            &[
                ("PAYLOAD_MEMBER_NAME", variant.payload_member_name()),
                ("FIELD_C_NAME", field.c_name()),
                ("FIELD_CODEC_NAME", field.codec_name()),
            ],
        ));
    }
    let wire_tag = variant.wire_tag().to_string();
    template::render(
        ERROR_READ_CASE,
        &[
            ("WIRE_TAG", &wire_tag),
            ("TAG_CONSTANT", variant.c_constant()),
            ("READ_FIELDS", &fields),
        ],
    )
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use uniffi_bindgen::ComponentInterface;

    use super::render;
    use crate::model::BindingsModel;

    #[test]
    fn renders_only_reachable_public_builtin_codecs() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};

            dictionary Example {
                u16 count;
                string name;
            };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let codec = render(&model);

        assert!(codec.contains("wallet_engine_private_write_u16"));
        assert!(codec.contains("wallet_engine_private_lower_string"));
        assert!(codec.contains("ffi_wallet_engine_rustbuffer_alloc"));
        assert!(!codec.contains("wallet_engine_private_write_i64"));
        Ok(())
    }

    #[test]
    fn renders_explicit_flat_enum_wire_mapping() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r#"
            namespace wallet_engine {};
            enum Network { "mainnet", "testnet" };
            "#,
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let codec = render(&model);

        assert!(codec.contains("wallet_engine_private_write_network"));
        assert!(codec.contains("case WALLET_ENGINE_NETWORK_MAINNET:"));
        assert!(codec.contains("wire_tag = INT32_C(1);"));
        assert!(codec.contains("case INT32_C(2):"));
        assert!(codec.contains("*out_value = WALLET_ENGINE_NETWORK_TESTNET;"));
        assert!(codec.contains("wallet_engine_private_lower_network"));
        assert!(codec.contains("wallet_engine_private_lift_network"));
        Ok(())
    }

    #[test]
    fn empty_component_does_not_render_unused_wire_helpers() -> Result<()> {
        let model = BindingsModel::from_components(&[ComponentInterface::new("wallet_engine")])?;

        assert!(render(&model).is_empty());
        Ok(())
    }

    #[test]
    fn renders_optional_tag_and_inner_codec_mapping() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            dictionary Example { u64? revision; };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let codec = render(&model);

        assert!(codec.contains("wallet_engine_private_write_optional_u64"));
        assert!(codec.contains("value.has_value ? 1u : 0u"));
        assert!(codec.contains("wallet_engine_private_write_u64(writer, value.value)"));
        assert!(codec.contains("wire_size += 8u;"));
        assert!(codec.contains("wallet_engine_private_lower_optional_u64"));
        assert!(codec.contains("wallet_engine_private_lift_optional_u64"));
        Ok(())
    }

    #[test]
    fn renders_custom_string_codec_as_a_typed_adapter() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            [Custom]
            typedef string Identifier;
            dictionary Example { Identifier value; };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let codec = render(&model);

        assert!(codec.contains("wallet_engine_private_write_identifier"));
        assert!(codec.contains("(WalletEngineStringView){value.data, value.len}"));
        assert!(codec.contains("wallet_engine_private_lower_string"));
        assert!(codec.contains("wallet_engine_private_lift_identifier"));
        Ok(())
    }

    #[test]
    fn renders_rich_error_tag_and_payload_codecs() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r#"
            namespace wallet_engine { [Throws=HostFailure] void call_host(); };
            enum Network { "mainnet", "testnet" };
            [Error]
            interface HostFailure {
                Cancelled();
                Failed(Network kind, string diagnostic);
            };
            "#,
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let codec = render(&model);

        assert!(codec.contains("wallet_engine_private_measure_host_failure"));
        assert!(codec.contains("case WALLET_ENGINE_HOST_FAILURE_FAILED:"));
        assert!(codec.contains("wallet_engine_private_write_i32(writer, INT32_C(2))"));
        assert!(codec.contains("value.payload.failed.diagnostic"));
        assert!(codec.contains("case INT32_C(1):"));
        assert!(codec.contains("out_value->tag = WALLET_ENGINE_HOST_FAILURE_CANCELLED;"));
        assert!(codec.contains("wallet_engine_private_lower_host_failure"));
        assert!(codec.contains("wallet_engine_private_lift_host_failure"));
        Ok(())
    }

    #[test]
    fn renders_fielded_enum_tag_and_payload_codecs() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            [Enum]
            interface SendAmount {
                Exact(u64 nanograms);
                All();
            };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let codec = render(&model);

        assert!(codec.contains("wallet_engine_private_measure_send_amount"));
        assert!(codec.contains("case WALLET_ENGINE_SEND_AMOUNT_EXACT:"));
        assert!(codec.contains("wallet_engine_private_write_i32(writer, INT32_C(1))"));
        assert!(codec.contains("value.payload.exact.nanograms"));
        assert!(codec.contains("case INT32_C(2):"));
        assert!(codec.contains("out_value->tag = WALLET_ENGINE_SEND_AMOUNT_ALL;"));
        assert!(codec.contains("wallet_engine_private_lower_send_amount"));
        assert!(codec.contains("wallet_engine_private_lift_send_amount"));
        Ok(())
    }

    #[test]
    fn renders_sequence_count_items_and_callback_arena() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            dictionary Example { sequence<u64> revisions; };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let codec = render(&model);

        assert!(codec.contains("wallet_engine_private_write_sequence_u64"));
        assert!(codec.contains("wallet_engine_private_write_i32(writer"));
        assert!(codec.contains("wallet_engine_private_read_u64"));
        assert!(codec.contains("wallet_engine_private_arena_alloc"));
        assert!(codec.contains("wire_size += value.len * 8u;"));
        Ok(())
    }

    #[test]
    fn renders_record_fields_in_declared_order() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            dictionary Example { string label; u64 revision; };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let codec = render(&model);
        let write_label = codec
            .find("wallet_engine_private_write_string(\n        writer,\n        value.label")
            .expect("label write should be rendered");
        let write_revision = codec
            .find("wallet_engine_private_write_u64(\n        writer,\n        value.revision")
            .expect("revision write should be rendered");

        assert!(write_label < write_revision);
        assert!(codec.contains("wallet_engine_private_measure_example"));
        assert!(codec.contains("wallet_engine_private_lift_example"));
        Ok(())
    }

    #[test]
    fn renders_dynamic_nested_compound_codecs_with_arena() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            dictionary Container {
                sequence<Item> items;
                Item? selected;
            };
            dictionary Item { string value; };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let codec = render(&model);
        let item = codec
            .find("wallet_engine_private_measure_item")
            .expect("Item codec should be rendered");
        let sequence = codec
            .find("wallet_engine_private_measure_sequence_item")
            .expect("Vec<Item> codec should be rendered");
        let optional = codec
            .find("wallet_engine_private_measure_optional_item")
            .expect("Option<Item> codec should be rendered");
        let container = codec
            .find("wallet_engine_private_measure_container")
            .expect("Container codec should be rendered");

        assert!(item < sequence && sequence < container);
        assert!(item < optional && optional < container);
        assert!(
            codec.contains("wallet_engine_private_measure_item(\n            value.data[index]")
        );
        assert!(
            codec.contains(
                "wallet_engine_private_read_item(\n            reader,\n            arena,"
            )
        );
        assert!(codec.contains("wallet_engine_private_measure_item(\n            value.value"));
        Ok(())
    }

    #[test]
    fn fieldless_record_uses_rustbuffer_without_unrelated_i32_codec() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            dictionary Empty {};
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let codec = render(&model);

        assert!(codec.contains("wallet_engine_private_rustbuffer_alloc"));
        assert!(codec.contains("wallet_engine_private_lower_empty"));
        assert!(!codec.contains("wallet_engine_private_write_i32"));
        Ok(())
    }
}
