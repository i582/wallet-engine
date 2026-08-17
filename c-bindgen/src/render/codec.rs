use std::fmt::Write as _;

use crate::{enum_map::FlatEnum, model::BindingsModel, template, type_map::BuiltinType};

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

pub(super) fn render(model: &BindingsModel) -> String {
    let mut output = if model.has_wire_types() {
        String::from(BASE)
    } else {
        String::new()
    };

    if model.has_builtin_type(BuiltinType::UInt8) || model.has_builtin_type(BuiltinType::Int8) {
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
        || model.has_flat_enums()
    {
        output.push_str(U32_WIRE_CODEC);
    }
    if model.has_builtin_type(BuiltinType::Int32) || model.has_flat_enums() {
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
    if model.has_builtin_type(BuiltinType::String) {
        output.push_str(STRING_CODEC);
    }
    if model.has_builtin_type(BuiltinType::Bytes) {
        output.push_str(BYTES_CODEC);
    }
    for enum_ in model.flat_enums() {
        output.push_str(&render_flat_enum(enum_));
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
}
