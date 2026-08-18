use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use uniffi_bindgen::{ComponentInterface, interface::Type};

use crate::{naming, type_registry::TypeRegistry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SemanticCustomType {
    uniffi_type: Type,
    rust_name: String,
    c_name: String,
    c_type_label: String,
    function_name: String,
    builtin_rust_name: String,
    builtin_c_name: String,
    builtin_function_name: String,
}

impl SemanticCustomType {
    pub(super) fn rust_name(&self) -> &str {
        &self.rust_name
    }

    pub(super) fn c_name(&self) -> &str {
        &self.c_name
    }

    pub(super) fn function_name(&self) -> &str {
        &self.function_name
    }

    pub(super) fn builtin_rust_name(&self) -> &str {
        &self.builtin_rust_name
    }

    pub(super) fn builtin_c_name(&self) -> &str {
        &self.builtin_c_name
    }

    pub(super) fn builtin_function_name(&self) -> &str {
        &self.builtin_function_name
    }
}

pub(super) fn collect_semantic_custom_types(
    component: &ComponentInterface,
    types: &mut TypeRegistry,
) -> Result<Vec<SemanticCustomType>> {
    let mut custom_types = BTreeMap::<String, Type>::new();
    for type_ in component.iter_local_types() {
        let Type::Custom {
            module_path,
            name,
            builtin,
        } = type_
        else {
            continue;
        };
        if module_path.split("::").next() != Some(component.crate_name()) {
            continue;
        }
        let definition = component
            .get_custom_type_definition(name)
            .with_context(|| format!("missing UniFFI custom type definition for {name}"))?;
        ensure!(
            definition.module_path.split("::").next() == Some(component.crate_name())
                && definition.builtin == **builtin,
            "conflicting UniFFI custom type definition for {name}"
        );
        if !matches!(&**builtin, Type::String) {
            continue;
        }
        if let Some(previous) = custom_types.insert(name.clone(), type_.clone()) {
            ensure!(
                previous == *type_,
                "conflicting reachable UniFFI custom types named {name}"
            );
        }
    }

    custom_types
        .into_values()
        .map(|uniffi_type| {
            let Type::Custom { name, builtin, .. } = &uniffi_type else {
                unreachable!("collector stores only custom types");
            };
            let builtin = types
                .resolve(builtin)
                .with_context(|| format!("unsupported builtin for UniFFI custom type {name}"))?;
            let c_type_label = format!("{name}View");
            let custom = SemanticCustomType {
                uniffi_type: uniffi_type.clone(),
                rust_name: name.clone(),
                c_name: naming::type_name(&c_type_label),
                c_type_label,
                function_name: naming::function_name(name),
                builtin_rust_name: builtin.rust_name().to_owned(),
                builtin_c_name: builtin.c_name().to_owned(),
                builtin_function_name: builtin.codec_name().to_owned(),
            };
            let registered = builtin.semantic_alias(
                custom.rust_name.clone(),
                custom.c_name.clone(),
                custom.c_type_label.clone(),
                custom.function_name.clone(),
            );
            types.register_type(&custom.uniffi_type, registered)?;
            Ok(custom)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use uniffi_bindgen::{ComponentInterface, interface::Type};

    use super::collect_semantic_custom_types;
    use crate::type_registry::TypeRegistry;

    #[test]
    fn collects_local_string_custom_types_with_semantic_names() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            [Custom]
            typedef string Identifier;
            dictionary Example { Identifier value; };
            ",
            "wallet_engine",
        )?;
        let mut types = TypeRegistry::collect(&component)?;
        let custom_types = collect_semantic_custom_types(&component, &mut types)?;

        assert_eq!(custom_types.len(), 1);
        assert_eq!(custom_types[0].rust_name(), "Identifier");
        assert_eq!(custom_types[0].c_name(), "WalletEngineIdentifierView");
        assert_eq!(custom_types[0].builtin_rust_name(), "String");
        assert_eq!(custom_types[0].builtin_function_name(), "string");
        let registered = types
            .resolve(&Type::Custom {
                module_path: "wallet_engine".to_owned(),
                name: "Identifier".to_owned(),
                builtin: Box::new(Type::String),
            })
            .context("Identifier should be registered")?;
        assert_eq!(registered.c_name(), "WalletEngineIdentifierView");
        Ok(())
    }
}
