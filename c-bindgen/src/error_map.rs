use anyhow::Result;
use uniffi_bindgen::{
    ComponentInterface,
    interface::{AsType, Type},
};

pub(super) use crate::tagged_enum::TaggedEnumType as ErrorType;
use crate::{tagged_enum::collect_tagged_enum_types, type_registry::TypeRegistry};

pub(super) fn collect_error_types(
    component: &ComponentInterface,
    types: &mut TypeRegistry,
) -> Result<Vec<ErrorType>> {
    let definitions = component
        .enum_definitions()
        .iter()
        .filter(|enum_| {
            let is_local = matches!(
                enum_.as_type(),
                Type::Enum { module_path, .. }
                    if module_path.split("::").next() == Some(component.crate_name())
            );
            is_local
                && !enum_.remote()
                && !enum_.is_flat()
                && component.is_name_used_as_error(enum_.name())
        })
        .collect();
    collect_tagged_enum_types(definitions, types, "declared error")
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use uniffi_bindgen::ComponentInterface;

    use super::collect_error_types;
    use crate::type_registry::{NestedWireSize, TypeRegistry};

    #[test]
    fn collects_supported_rich_errors_and_skips_flat_or_unresolved_errors() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r#"
            namespace wallet_engine {
                [Throws=HostFailure] void call_host();
                [Throws=FlatFailure] void fail_flat();
                [Throws=UnsupportedFailure] void fail_unsupported();
            };

            enum Network { "mainnet", "testnet" };
            [Enum]
            interface Payload { Value(u64 value); };

            [Error]
            interface HostFailure {
                Cancelled();
                Failed(Network kind, string diagnostic);
            };
            [Error]
            enum FlatFailure { "failed" };
            [Error]
            interface UnsupportedFailure { Failed(Payload payload); };
            "#,
            "wallet_engine",
        )?;
        let mut types = TypeRegistry::collect(&component)?;
        let errors = collect_error_types(&component, &mut types)?;

        assert_eq!(errors.len(), 1);
        let error = &errors[0];
        assert_eq!(error.rust_name(), "HostFailure");
        assert_eq!(error.c_name(), "WalletEngineHostFailure");
        assert_eq!(error.tag_c_name(), "WalletEngineHostFailureTag");
        assert_eq!(error.variants()[0].public_value(), 0);
        assert_eq!(error.variants()[0].wire_tag(), 1);
        assert!(error.variants()[0].payload_c_name().is_none());
        let failed = error
            .variants()
            .get(1)
            .context("Failed variant should be present")?;
        assert_eq!(failed.c_constant(), "WALLET_ENGINE_HOST_FAILURE_FAILED");
        assert_eq!(
            failed.payload_c_name(),
            Some("WalletEngineHostFailureFailedPayload")
        );
        assert_eq!(failed.payload_member_name(), "failed");
        assert_eq!(failed.fields()[0].c_type_name(), "WalletEngineNetwork");
        assert_eq!(
            failed.fields()[1].nested_wire_size(),
            NestedWireSize::LengthPrefixedView
        );
        Ok(())
    }
}
