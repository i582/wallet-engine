use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use uniffi_bindgen::{ComponentInterface, interface::Type};

use crate::{
    naming,
    type_registry::{NestedWireSize, RegisteredType, TypeRegistry},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OptionalType {
    uniffi_type: Type,
    rust_name: String,
    c_name: String,
    c_type_label: String,
    function_name: String,
    inner_rust_name: String,
    inner_c_name: String,
    inner_function_name: String,
    inner_wire_size: NestedWireSize,
}

impl OptionalType {
    pub(super) fn rust_name(&self) -> &str {
        &self.rust_name
    }

    pub(super) fn c_name(&self) -> &str {
        &self.c_name
    }

    pub(super) fn function_name(&self) -> &str {
        &self.function_name
    }

    pub(super) fn inner_rust_name(&self) -> &str {
        &self.inner_rust_name
    }

    pub(super) fn inner_c_name(&self) -> &str {
        &self.inner_c_name
    }

    pub(super) fn inner_function_name(&self) -> &str {
        &self.inner_function_name
    }

    pub(super) const fn inner_wire_size(&self) -> NestedWireSize {
        self.inner_wire_size
    }

    pub(super) const fn uniffi_type(&self) -> &Type {
        &self.uniffi_type
    }

    pub(super) fn registered_type(&self) -> RegisteredType {
        RegisteredType::compound(
            self.rust_name.clone(),
            self.c_name.clone(),
            self.c_type_label.clone(),
            self.function_name.clone(),
            false,
        )
    }
}

pub(super) fn collect_optional_types(
    component: &ComponentInterface,
    types: &TypeRegistry,
) -> Result<Vec<OptionalType>> {
    let mut optionals = component
        .iter_local_types()
        .filter_map(|type_| match type_ {
            Type::Optional { inner_type } => optional_type(type_, inner_type, types),
            _ => None,
        })
        .collect::<Vec<_>>();
    optionals.sort_by(|left, right| left.rust_name.cmp(&right.rust_name));

    let mut c_names = BTreeSet::new();
    let mut function_names = BTreeSet::new();
    for optional in &optionals {
        ensure!(
            c_names.insert(optional.c_name.clone()),
            "optional types produce duplicate C type {}",
            optional.c_name
        );
        ensure!(
            function_names.insert(optional.function_name.clone()),
            "optional types produce duplicate codec name {}",
            optional.function_name
        );
    }
    Ok(optionals)
}

fn optional_type(
    uniffi_type: &Type,
    inner_type: &Type,
    types: &TypeRegistry,
) -> Option<OptionalType> {
    let inner = types.resolve(inner_type)?;
    let rust_name = format!("Option<{}>", inner.rust_name());
    let c_type_label = format!("Optional{}", inner.c_type_label());
    let c_name = naming::type_name(&c_type_label);
    let function_name = format!("optional_{}", inner.codec_name());

    Some(OptionalType {
        uniffi_type: uniffi_type.clone(),
        rust_name,
        c_name,
        c_type_label,
        function_name,
        inner_rust_name: inner.rust_name().to_owned(),
        inner_c_name: inner.c_name().to_owned(),
        inner_function_name: inner.codec_name().to_owned(),
        inner_wire_size: inner.nested_wire_size(),
    })
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use uniffi_bindgen::ComponentInterface;

    use super::collect_optional_types;
    use crate::type_registry::{NestedWireSize, TypeRegistry};

    #[test]
    fn collects_options_with_supported_inner_types() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r#"
            namespace wallet_engine {};

            dictionary Example {
                u64? revision;
                string? name;
                Network? network;
                Nested? nested;
            };
            dictionary Nested { string value; };
            enum Network { "mainnet", "testnet" };
            "#,
            "wallet_engine",
        )?;
        let types = TypeRegistry::collect(&component)?;
        let optionals = collect_optional_types(&component, &types)?;

        assert_eq!(optionals.len(), 3);
        let network = optionals
            .iter()
            .find(|optional| optional.rust_name() == "Option<Network>")
            .context("Option<Network> should be supported")?;
        assert_eq!(network.c_name(), "WalletEngineOptionalNetwork");
        assert_eq!(network.inner_c_name(), "WalletEngineNetwork");
        assert_eq!(network.inner_wire_size(), NestedWireSize::Fixed(4));
        let string = optionals
            .iter()
            .find(|optional| optional.rust_name() == "Option<String>")
            .context("Option<String> should be supported")?;
        assert_eq!(string.c_name(), "WalletEngineOptionalStringView");
        assert_eq!(string.inner_wire_size(), NestedWireSize::LengthPrefixedView);
        Ok(())
    }
}
