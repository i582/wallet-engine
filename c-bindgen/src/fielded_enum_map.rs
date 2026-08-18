use anyhow::Result;
use uniffi_bindgen::{
    ComponentInterface,
    interface::{AsType, Type},
};

pub(super) use crate::tagged_enum::TaggedEnumType as FieldedEnumType;
use crate::{tagged_enum::collect_tagged_enum_types, type_registry::TypeRegistry};

pub(super) fn collect_fielded_enum_types(
    component: &ComponentInterface,
    types: &mut TypeRegistry,
) -> Result<Vec<FieldedEnumType>> {
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
                && !component.is_name_used_as_error(enum_.name())
        })
        .collect();
    collect_tagged_enum_types(definitions, types, "fielded enum")
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use uniffi_bindgen::ComponentInterface;

    use super::collect_fielded_enum_types;
    use crate::type_registry::TypeRegistry;

    #[test]
    fn collects_non_error_enum_with_payload() -> Result<()> {
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
        let mut types = TypeRegistry::collect(&component)?;
        let enums = collect_fielded_enum_types(&component, &mut types)?;

        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].rust_name(), "SendAmount");
        assert_eq!(enums[0].c_name(), "WalletEngineSendAmount");
        assert_eq!(enums[0].variants()[0].public_value(), 0);
        assert_eq!(enums[0].variants()[0].wire_tag(), 1);
        let exact = enums[0]
            .variants()
            .first()
            .context("Exact should be present")?;
        assert_eq!(exact.fields()[0].c_name(), "nanograms");
        assert_eq!(exact.fields()[0].c_type_name(), "uint64_t");
        assert!(enums[0].variants()[1].payload_c_name().is_none());
        Ok(())
    }
}
