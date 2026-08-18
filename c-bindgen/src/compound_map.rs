use anyhow::Result;
use uniffi_bindgen::ComponentInterface;

use crate::{
    optional_map::{OptionalType, collect_optional_types},
    record_map::{RecordType, collect_record_types},
    sequence_map::{SequenceType, collect_sequence_types},
    type_registry::TypeRegistry,
};

#[derive(Debug)]
pub(super) struct CompoundTypes {
    optionals: Vec<OptionalType>,
    sequences: Vec<SequenceType>,
    records: Vec<RecordType>,
    order: Vec<CompoundTypeIndex>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompoundTypeIndex {
    Optional(usize),
    Sequence(usize),
    Record(usize),
}

#[derive(Debug, Clone, Copy)]
pub(super) enum CompoundTypeRef<'a> {
    Optional(&'a OptionalType),
    Sequence(&'a SequenceType),
    Record(&'a RecordType),
}

impl CompoundTypes {
    pub(super) fn collect(
        component: &ComponentInterface,
        types: &mut TypeRegistry,
    ) -> Result<Self> {
        let mut collected = Self {
            optionals: Vec::new(),
            sequences: Vec::new(),
            records: Vec::new(),
            order: Vec::new(),
        };

        loop {
            let previous_len = collected.order.len();

            for optional in collect_optional_types(component, types)? {
                types.register_type(optional.uniffi_type(), optional.registered_type())?;
                collected
                    .order
                    .push(CompoundTypeIndex::Optional(collected.optionals.len()));
                collected.optionals.push(optional);
            }
            for sequence in collect_sequence_types(component, types)? {
                types.register_type(sequence.uniffi_type(), sequence.registered_type())?;
                collected
                    .order
                    .push(CompoundTypeIndex::Sequence(collected.sequences.len()));
                collected.sequences.push(sequence);
            }
            for record in collect_record_types(component, types)? {
                collected
                    .order
                    .push(CompoundTypeIndex::Record(collected.records.len()));
                collected.records.push(record);
            }

            if collected.order.len() == previous_len {
                return Ok(collected);
            }
        }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = CompoundTypeRef<'_>> {
        self.order.iter().map(|index| match *index {
            CompoundTypeIndex::Optional(index) => CompoundTypeRef::Optional(&self.optionals[index]),
            CompoundTypeIndex::Sequence(index) => CompoundTypeRef::Sequence(&self.sequences[index]),
            CompoundTypeIndex::Record(index) => CompoundTypeRef::Record(&self.records[index]),
        })
    }

    pub(super) fn optionals(&self) -> &[OptionalType] {
        &self.optionals
    }

    pub(super) fn sequences(&self) -> &[SequenceType] {
        &self.sequences
    }

    pub(super) fn records(&self) -> &[RecordType] {
        &self.records
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use uniffi_bindgen::ComponentInterface;

    use super::{CompoundTypeRef, CompoundTypes};
    use crate::type_registry::TypeRegistry;

    #[test]
    fn collects_nested_compounds_in_dependency_order() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};

            dictionary Item { string value; };
            dictionary Container {
                sequence<Item> items;
                Item? selected;
            };
            ",
            "wallet_engine",
        )?;
        let mut types = TypeRegistry::collect(&component)?;
        let compounds = CompoundTypes::collect(&component, &mut types)?;
        let labels = compounds
            .iter()
            .map(|type_| match type_ {
                CompoundTypeRef::Optional(value) => value.rust_name().to_owned(),
                CompoundTypeRef::Sequence(value) => value.rust_name().to_owned(),
                CompoundTypeRef::Record(value) => value.rust_name().to_owned(),
            })
            .collect::<Vec<_>>();

        let item = labels
            .iter()
            .position(|label| label == "Item")
            .context("Item should be collected")?;
        let optional = labels
            .iter()
            .position(|label| label == "Option<Item>")
            .context("Option<Item> should be collected")?;
        let sequence = labels
            .iter()
            .position(|label| label == "Vec<Item>")
            .context("Vec<Item> should be collected")?;
        let container = labels
            .iter()
            .position(|label| label == "Container")
            .context("Container should be collected")?;

        assert!(item < optional);
        assert!(item < sequence);
        assert!(optional < container);
        assert!(sequence < container);
        Ok(())
    }
}
