use std::fmt::Write as _;

use crate::{
    enum_map::FlatEnum, model::BindingsModel, optional_map::OptionalType,
    sequence_map::SequenceType, template, type_map::BuiltinType, type_registry::NestedWireSize,
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
const OPTIONAL_CODEC: &str = include_str!("../../templates/codecs/optional.c.tmpl");
const ARENA_RUNTIME: &str = include_str!("../../templates/codecs/arena.c.tmpl");
const SEQUENCE_CODEC: &str = include_str!("../../templates/codecs/sequence.c.tmpl");
const SEQUENCE_FIXED_MEASURE: &str =
    include_str!("../../templates/codecs/sequence_measure_fixed.c.tmpl");
const SEQUENCE_LENGTH_PREFIXED_VIEW_MEASURE: &str =
    include_str!("../../templates/codecs/sequence_measure_length_prefixed_view.c.tmpl");

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
    if model.has_sequence_types() {
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
    for optional in model.optional_types() {
        output.push_str(&render_optional(optional));
    }
    for sequence in model.sequence_types() {
        output.push_str(&render_sequence(sequence));
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
    let some_wire_size = match optional.inner_wire_size() {
        NestedWireSize::Fixed(inner_size) => {
            format!("        wire_size = {}u;\n", inner_size + 1)
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
    };

    template::render(
        OPTIONAL_CODEC,
        &[
            ("FUNCTION_NAME", optional.function_name()),
            ("C_NAME", optional.c_name()),
            ("INNER_FUNCTION_NAME", optional.inner_function_name()),
            ("SOME_WIRE_SIZE", &some_wire_size),
        ],
    )
}

fn render_sequence(sequence: &SequenceType) -> String {
    let (minimum_inner_wire_size, measure_items) = match sequence.inner_wire_size() {
        NestedWireSize::Fixed(inner_size) => {
            let inner_size = inner_size.to_string();
            let measure_items =
                template::render(SEQUENCE_FIXED_MEASURE, &[("INNER_WIRE_SIZE", &inner_size)]);
            (inner_size, measure_items)
        }
        NestedWireSize::LengthPrefixedView => (
            String::from("4"),
            String::from(SEQUENCE_LENGTH_PREFIXED_VIEW_MEASURE),
        ),
    };

    template::render(
        SEQUENCE_CODEC,
        &[
            ("FUNCTION_NAME", sequence.function_name()),
            ("C_NAME", sequence.c_name()),
            ("INNER_C_NAME", sequence.inner_c_name()),
            ("INNER_FUNCTION_NAME", sequence.inner_function_name()),
            ("MIN_INNER_WIRE_SIZE", &minimum_inner_wire_size),
            ("MEASURE_ITEMS", &measure_items),
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
        assert!(codec.contains("wire_size = 9u;"));
        assert!(codec.contains("wallet_engine_private_lower_optional_u64"));
        assert!(codec.contains("wallet_engine_private_lift_optional_u64"));
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
}
