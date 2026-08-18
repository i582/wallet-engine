use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use uniffi_bindgen::{ComponentInterface, interface::Type};

use crate::{
    naming,
    type_registry::{NestedWireSize, RegisteredType, TypeRegistry},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SequenceType {
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

impl SequenceType {
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
            true,
        )
    }
}

pub(super) fn collect_sequence_types(
    component: &ComponentInterface,
    types: &TypeRegistry,
) -> Result<Vec<SequenceType>> {
    let mut sequences = component
        .iter_local_types()
        .filter_map(|type_| match type_ {
            Type::Sequence { inner_type } => sequence_type(type_, inner_type, types),
            _ => None,
        })
        .collect::<Vec<_>>();
    sequences.sort_by(|left, right| left.rust_name.cmp(&right.rust_name));

    let mut rust_names = BTreeSet::new();
    let mut c_names = BTreeSet::new();
    let mut function_names = BTreeSet::new();
    sequences.retain(|sequence| rust_names.insert(sequence.rust_name.clone()));
    for sequence in &sequences {
        ensure!(
            c_names.insert(sequence.c_name.clone()),
            "sequence types produce duplicate C type {}",
            sequence.c_name
        );
        ensure!(
            function_names.insert(sequence.function_name.clone()),
            "sequence types produce duplicate codec name {}",
            sequence.function_name
        );
    }
    Ok(sequences)
}

fn sequence_type(
    uniffi_type: &Type,
    inner_type: &Type,
    types: &TypeRegistry,
) -> Option<SequenceType> {
    let inner = types.resolve(inner_type)?;
    let inner_label = inner
        .c_type_label()
        .strip_suffix("View")
        .unwrap_or_else(|| inner.c_type_label());

    let c_type_label = format!("{inner_label}ListView");

    Some(SequenceType {
        uniffi_type: uniffi_type.clone(),
        rust_name: format!("Vec<{}>", inner.rust_name()),
        c_name: naming::type_name(&c_type_label),
        c_type_label,
        function_name: format!("sequence_{}", inner.codec_name()),
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

    use super::collect_sequence_types;
    use crate::type_registry::{NestedWireSize, TypeRegistry};

    #[test]
    fn collects_sequences_with_supported_inner_types() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r#"
            namespace wallet_engine {};

            dictionary Example {
                sequence<u64> revisions;
                sequence<string> names;
                sequence<Network> networks;
                sequence<Nested> nested;
            };
            dictionary Nested { string value; };
            enum Network { "mainnet", "testnet" };
            "#,
            "wallet_engine",
        )?;
        let types = TypeRegistry::collect(&component)?;
        let sequences = collect_sequence_types(&component, &types)?;

        assert_eq!(sequences.len(), 3);
        let integers = sequences
            .iter()
            .find(|sequence| sequence.rust_name() == "Vec<u64>")
            .context("Vec<u64> should be supported")?;
        assert_eq!(integers.c_name(), "WalletEngineU64ListView");
        assert_eq!(integers.inner_c_name(), "uint64_t");
        assert_eq!(integers.inner_wire_size(), NestedWireSize::Fixed(8));

        let strings = sequences
            .iter()
            .find(|sequence| sequence.rust_name() == "Vec<String>")
            .context("Vec<String> should be supported")?;
        assert_eq!(strings.c_name(), "WalletEngineStringListView");
        assert_eq!(
            strings.inner_wire_size(),
            NestedWireSize::LengthPrefixedView
        );
        Ok(())
    }
}
