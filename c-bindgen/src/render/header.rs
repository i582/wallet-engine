use std::fmt::Write as _;

use crate::{error_map::ErrorType, model::BindingsModel, template, type_map::BuiltinType};

const HEADER_TEMPLATE: &str = include_str!("../../templates/header.h.tmpl");
const STRING_VIEW_TEMPLATE: &str = include_str!("../../templates/types/string_view.h.tmpl");
const BYTES_VIEW_TEMPLATE: &str = include_str!("../../templates/types/bytes_view.h.tmpl");
const FLAT_ENUM_TEMPLATE: &str = include_str!("../../templates/types/flat_enum.h.tmpl");
const OPTIONAL_TEMPLATE: &str = include_str!("../../templates/types/optional.h.tmpl");
const SEQUENCE_TEMPLATE: &str = include_str!("../../templates/types/sequence.h.tmpl");
const RECORD_TEMPLATE: &str = include_str!("../../templates/types/record.h.tmpl");
const RECORD_FIELD_TEMPLATE: &str = include_str!("../../templates/types/record_field.h.tmpl");
const EMPTY_RECORD_FIELD_TEMPLATE: &str =
    include_str!("../../templates/types/empty_record_field.h.tmpl");
const ERROR_TEMPLATE: &str = include_str!("../../templates/types/error.h.tmpl");
const ERROR_PAYLOAD_STRUCT_TEMPLATE: &str =
    include_str!("../../templates/types/error_payload_struct.h.tmpl");
const ERROR_PAYLOAD_MEMBER_TEMPLATE: &str =
    include_str!("../../templates/types/error_payload_member.h.tmpl");

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
    for sequence in model.sequence_types() {
        type_declarations.push_str(&template::render(
            SEQUENCE_TEMPLATE,
            &[
                ("RUST_NAME", sequence.rust_name()),
                ("C_NAME", sequence.c_name()),
                ("INNER_C_NAME", sequence.inner_c_name()),
            ],
        ));
    }
    for (index, record) in model.record_types().iter().enumerate() {
        if index != 0 {
            type_declarations.push('\n');
        }
        let fields = if record.fields().is_empty() {
            String::from(EMPTY_RECORD_FIELD_TEMPLATE)
        } else {
            record
                .fields()
                .iter()
                .map(|field| {
                    template::render(
                        RECORD_FIELD_TEMPLATE,
                        &[
                            ("FIELD_C_TYPE", field.c_type_name()),
                            ("FIELD_C_NAME", field.c_name()),
                        ],
                    )
                })
                .collect()
        };
        type_declarations.push_str(&template::render(
            RECORD_TEMPLATE,
            &[
                ("RUST_NAME", record.rust_name()),
                ("C_NAME", record.c_name()),
                ("FIELDS", &fields),
            ],
        ));
    }
    for error in model.error_types() {
        type_declarations.push_str(&render_error(error));
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

fn render_error(error: &ErrorType) -> String {
    let mut tag_constants = String::new();
    let mut payload_structs = String::new();
    let mut payload_members = String::new();
    for (index, variant) in error.variants().iter().enumerate() {
        if index != 0 {
            tag_constants.push('\n');
        }
        let _ = write!(
            tag_constants,
            "#define {} (({}){}u)",
            variant.c_constant(),
            error.tag_c_name(),
            variant.public_value(),
        );
        let Some(payload_c_name) = variant.payload_c_name() else {
            continue;
        };
        let payload_member_name = variant.payload_member_name();
        let fields = variant
            .fields()
            .iter()
            .map(|field| {
                template::render(
                    RECORD_FIELD_TEMPLATE,
                    &[
                        ("FIELD_C_TYPE", field.c_type_name()),
                        ("FIELD_C_NAME", field.c_name()),
                    ],
                )
            })
            .collect::<String>();
        payload_structs.push_str(&template::render(
            ERROR_PAYLOAD_STRUCT_TEMPLATE,
            &[("PAYLOAD_C_NAME", payload_c_name), ("FIELDS", &fields)],
        ));
        payload_members.push_str(&template::render(
            ERROR_PAYLOAD_MEMBER_TEMPLATE,
            &[
                ("PAYLOAD_C_NAME", payload_c_name),
                ("PAYLOAD_MEMBER_NAME", payload_member_name),
            ],
        ));
    }

    template::render(
        ERROR_TEMPLATE,
        &[
            ("RUST_NAME", error.rust_name()),
            ("TAG_C_NAME", error.tag_c_name()),
            ("TAG_CONSTANTS", &tag_constants),
            ("PAYLOAD_STRUCTS", &payload_structs),
            ("PAYLOAD_C_NAME", error.payload_c_name()),
            ("PAYLOAD_MEMBERS", &payload_members),
            ("C_NAME", error.c_name()),
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

    #[test]
    fn renders_sequence_as_a_borrowed_list_view() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            dictionary Example { sequence<string> names; };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let header = render(&model);

        assert!(header.contains("typedef struct WalletEngineStringListView"));
        assert!(header.contains("const WalletEngineStringView *data;"));
        assert!(header.contains("size_t len;"));
        Ok(())
    }

    #[test]
    fn renders_records_in_dependency_order() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            dictionary Outer { Inner inner; u64 revision; };
            dictionary Inner { string label; };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let header = render(&model);
        let inner = header
            .find("typedef struct WalletEngineInnerView")
            .expect("Inner view should be rendered");
        let outer = header
            .find("typedef struct WalletEngineOuterView")
            .expect("Outer view should be rendered");

        assert!(inner < outer);
        assert!(header.contains("WalletEngineInnerView inner;"));
        assert!(header.contains("uint64_t revision;"));
        assert!(
            header.contains(
                "} WalletEngineInnerView;\n\n/* A borrowed view of Rust record `Outer`. */"
            )
        );
        Ok(())
    }

    #[test]
    fn renders_rich_error_as_tag_and_named_payload_union() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine { [Throws=HostFailure] void call_host(); };
            [Error]
            interface HostFailure {
                Cancelled();
                Failed(u64 code, string diagnostic);
            };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let header = render(&model);

        assert!(header.contains("typedef uint32_t WalletEngineHostFailureTag;"));
        assert!(header.contains(
            "#define WALLET_ENGINE_HOST_FAILURE_CANCELLED ((WalletEngineHostFailureTag)0u)"
        ));
        assert!(header.contains("typedef struct WalletEngineHostFailureFailedPayload"));
        assert!(header.contains("uint64_t code;"));
        assert!(header.contains("WalletEngineStringView diagnostic;"));
        assert!(header.contains("typedef union WalletEngineHostFailurePayload"));
        assert!(header.contains("WalletEngineHostFailureFailedPayload failed;"));
        assert!(header.contains("WalletEngineHostFailureTag tag;"));
        Ok(())
    }
}
