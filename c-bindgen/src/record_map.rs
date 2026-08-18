use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use uniffi_bindgen::{
    ComponentInterface,
    interface::{AsType, Record, Type},
};

use crate::{
    naming,
    type_registry::{NestedWireSize, RegisteredType, TypeRegistry},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordType {
    uniffi_type: Type,
    rust_name: String,
    c_name: String,
    c_type_label: String,
    function_name: String,
    minimum_wire_size: usize,
    fields: Vec<RecordField>,
}

impl RecordType {
    pub(super) fn rust_name(&self) -> &str {
        &self.rust_name
    }

    pub(super) fn c_name(&self) -> &str {
        &self.c_name
    }

    pub(super) fn function_name(&self) -> &str {
        &self.function_name
    }

    pub(super) fn fields(&self) -> &[RecordField] {
        &self.fields
    }

    fn registered_type(&self) -> RegisteredType {
        RegisteredType::compound(
            self.rust_name.clone(),
            self.c_name.clone(),
            self.c_type_label.clone(),
            self.function_name.clone(),
            self.minimum_wire_size,
            true,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordField {
    rust_name: String,
    c_name: String,
    rust_type_name: String,
    c_type_name: String,
    codec_name: String,
    nested_wire_size: NestedWireSize,
    minimum_wire_size: usize,
    read_needs_arena: bool,
}

impl RecordField {
    pub(super) fn rust_name(&self) -> &str {
        &self.rust_name
    }

    pub(super) fn c_name(&self) -> &str {
        &self.c_name
    }

    pub(super) fn rust_type_name(&self) -> &str {
        &self.rust_type_name
    }

    pub(super) fn c_type_name(&self) -> &str {
        &self.c_type_name
    }

    pub(super) fn codec_name(&self) -> &str {
        &self.codec_name
    }

    pub(super) const fn nested_wire_size(&self) -> NestedWireSize {
        self.nested_wire_size
    }

    pub(super) const fn read_needs_arena(&self) -> bool {
        self.read_needs_arena
    }

    pub(super) const fn minimum_wire_size(&self) -> usize {
        self.minimum_wire_size
    }
}

pub(super) fn collect_record_types(
    component: &ComponentInterface,
    types: &mut TypeRegistry,
) -> Result<Vec<RecordType>> {
    let mut remaining = component
        .record_definitions()
        .iter()
        .filter(|record| {
            let is_local = matches!(
                record.as_type(),
                Type::Record { module_path, .. }
                    if module_path.split("::").next() == Some(component.crate_name())
            );
            is_local && !record.remote() && types.resolve(&record.as_type()).is_none()
        })
        .collect::<Vec<_>>();
    remaining.sort_by(|left, right| left.name().cmp(right.name()));

    let mut records = Vec::new();
    loop {
        let previous_len = remaining.len();
        let mut pending = Vec::new();
        for record in remaining {
            let Some(record_type) = record_type(record, types)? else {
                pending.push(record);
                continue;
            };
            types.register_type(&record_type.uniffi_type, record_type.registered_type())?;
            records.push(record_type);
        }
        if pending.is_empty() || pending.len() == previous_len {
            break;
        }
        remaining = pending;
    }
    Ok(records)
}

fn record_type(record: &Record, types: &TypeRegistry) -> Result<Option<RecordType>> {
    let mut fields = Vec::new();
    let mut c_field_names = BTreeSet::new();
    for field in record.fields() {
        let field_type = field.as_type();
        let Some(registered) = types.resolve(&field_type) else {
            return Ok(None);
        };
        let c_name = naming::field_name(field.name());
        ensure!(
            c_field_names.insert(c_name.clone()),
            "record {} produces duplicate C field {c_name}",
            record.name()
        );
        fields.push(RecordField {
            rust_name: field.name().to_owned(),
            c_name,
            rust_type_name: registered.rust_name().to_owned(),
            c_type_name: registered.c_name().to_owned(),
            codec_name: registered.codec_name().to_owned(),
            nested_wire_size: registered.nested_wire_size(),
            minimum_wire_size: registered.minimum_wire_size(),
            read_needs_arena: registered.read_needs_arena(),
        });
    }
    let c_type_label = format!("{}View", record.name());
    let minimum_wire_size = fields.iter().try_fold(0usize, |total, field| {
        total.checked_add(field.minimum_wire_size()).ok_or_else(|| {
            anyhow::anyhow!(
                "record {} minimum wire size overflows size_t",
                record.name()
            )
        })
    })?;

    Ok(Some(RecordType {
        uniffi_type: record.as_type(),
        rust_name: record.name().to_owned(),
        c_name: naming::type_name(&c_type_label),
        c_type_label,
        function_name: naming::function_name(record.name()),
        minimum_wire_size,
        fields,
    }))
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use uniffi_bindgen::ComponentInterface;

    use super::collect_record_types;
    use crate::{
        optional_map::collect_optional_types, sequence_map::collect_sequence_types,
        type_registry::TypeRegistry,
    };

    #[test]
    fn collects_supported_records_in_dependency_order() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r#"
            namespace wallet_engine {};

            dictionary Outer {
                Inner inner;
                sequence<string> names;
                u64? revision;
                Network network;
            };
            dictionary Inner { string label; };
            dictionary Unsupported { Payload payload; };
            enum Network { "mainnet", "testnet" };
            [Enum]
            interface Payload { Value(u64 value); };
            "#,
            "wallet_engine",
        )?;
        let mut types = TypeRegistry::collect(&component)?;
        let optionals = collect_optional_types(&component, &types)?;
        let sequences = collect_sequence_types(&component, &types)?;
        for optional in &optionals {
            types.register_type(optional.uniffi_type(), optional.registered_type())?;
        }
        for sequence in &sequences {
            types.register_type(sequence.uniffi_type(), sequence.registered_type())?;
        }

        let records = collect_record_types(&component, &mut types)?;
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].rust_name(), "Inner");
        assert_eq!(records[1].rust_name(), "Outer");
        let outer = records
            .iter()
            .find(|record| record.rust_name() == "Outer")
            .context("Outer should be supported")?;
        assert_eq!(outer.c_name(), "WalletEngineOuterView");
        assert_eq!(outer.fields()[0].c_type_name(), "WalletEngineInnerView");
        assert_eq!(
            outer.fields()[1].c_type_name(),
            "WalletEngineStringListView"
        );
        assert_eq!(outer.fields()[2].c_type_name(), "WalletEngineOptionalU64");
        Ok(())
    }
}
