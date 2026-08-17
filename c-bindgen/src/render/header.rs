use std::fmt::Write as _;

use crate::{model::BindingsModel, template, type_map::BuiltinType};

const HEADER_TEMPLATE: &str = include_str!("../../templates/header.h.tmpl");
const STRING_VIEW_TEMPLATE: &str = include_str!("../../templates/types/string_view.h.tmpl");
const BYTES_VIEW_TEMPLATE: &str = include_str!("../../templates/types/bytes_view.h.tmpl");
const FLAT_ENUM_TEMPLATE: &str = include_str!("../../templates/types/flat_enum.h.tmpl");
const OPTIONAL_TEMPLATE: &str = include_str!("../../templates/types/optional.h.tmpl");

pub(super) fn render(model: &BindingsModel) -> String {
    let abi_version = model.abi_version().to_string();
    let uniffi_contract_version = model.uniffi_contract_version().to_string();
    let mut type_declarations = String::new();

    if model.has_builtin_type(BuiltinType::String) {
        type_declarations.push_str(STRING_VIEW_TEMPLATE);
    }
    if model.has_builtin_type(BuiltinType::Bytes) {
        type_declarations.push_str(BYTES_VIEW_TEMPLATE);
    }
    for enum_ in model.flat_enums() {
        let mut variants = String::new();
        for (index, variant) in enum_.variants().iter().enumerate() {
            if index != 0 {
                variants.push('\n');
            }
            let _ = write!(
                variants,
                "#define {} (({}){}u)",
                variant.c_constant(),
                enum_.c_name(),
                variant.public_value(),
            );
        }
        type_declarations.push_str(&template::render(
            FLAT_ENUM_TEMPLATE,
            &[
                ("RUST_NAME", enum_.rust_name()),
                ("C_NAME", enum_.c_name()),
                ("VARIANTS", &variants),
            ],
        ));
    }
    for optional in model.optional_types() {
        type_declarations.push_str(&template::render(
            OPTIONAL_TEMPLATE,
            &[
                ("RUST_NAME", optional.rust_name()),
                ("C_NAME", optional.c_name()),
                ("INNER_C_NAME", optional.inner_c_name()),
            ],
        ));
    }

    template::render(
        HEADER_TEMPLATE,
        &[
            ("ABI_VERSION", &abi_version),
            ("UNIFFI_CONTRACT_VERSION", &uniffi_contract_version),
            ("TYPE_DECLARATIONS", &type_declarations),
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
    fn renders_a_self_contained_experimental_header() -> Result<()> {
        let component = ComponentInterface::new("wallet_engine");
        let model = BindingsModel::from_components(&[component])?;
        let header = render(&model);

        assert!(header.starts_with("#ifndef WALLET_ENGINE_H\n"));
        assert!(header.contains("#define WALLET_ENGINE_ABI_VERSION 0u\n"));
        assert!(header.contains("typedef uint32_t WalletEngineAbiStatus;"));
        assert!(header.contains("WALLET_ENGINE_ABI_STATUS_INVALID_UTF8"));
        assert!(header.contains("wallet_engine_abi_version(void);"));
        assert!(header.ends_with("#endif /* WALLET_ENGINE_H */\n"));
        Ok(())
    }

    #[test]
    fn renders_borrowed_string_and_byte_views_when_used() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};

            dictionary Example {
                string name;
                bytes payload;
            };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let header = render(&model);

        assert!(header.contains("typedef struct WalletEngineStringView"));
        assert!(header.contains("typedef struct WalletEngineBytesView"));
        Ok(())
    }

    #[test]
    fn renders_flat_enum_with_stable_public_values() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r#"
            namespace wallet_engine {};
            enum Network { "mainnet", "testnet" };
            "#,
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let header = render(&model);

        assert!(header.contains("typedef uint32_t WalletEngineNetwork;"));
        assert!(header.contains("#define WALLET_ENGINE_NETWORK_MAINNET ((WalletEngineNetwork)0u)"));
        assert!(header.contains("#define WALLET_ENGINE_NETWORK_TESTNET ((WalletEngineNetwork)1u)"));
        Ok(())
    }

    #[test]
    fn renders_optional_with_an_explicit_presence_flag() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            dictionary Example { u64? revision; };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let header = render(&model);

        assert!(header.contains("typedef struct WalletEngineOptionalU64"));
        assert!(header.contains("bool has_value;"));
        assert!(header.contains("uint64_t value;"));
        Ok(())
    }
}
