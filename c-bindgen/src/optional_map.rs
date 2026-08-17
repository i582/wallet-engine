use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use uniffi_bindgen::{ComponentInterface, interface::Type};

use crate::{enum_map::FlatEnum, naming, type_map::BuiltinType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OptionalWireSize {
    Fixed(usize),
    LengthPrefixedView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OptionalType {
    rust_name: String,
    c_name: String,
    function_name: String,
    inner_rust_name: String,
    inner_c_name: String,
    inner_function_name: String,
    inner_wire_size: OptionalWireSize,
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

    pub(super) const fn inner_wire_size(&self) -> OptionalWireSize {
        self.inner_wire_size
    }
}

pub(super) fn collect_optional_types(
    component: &ComponentInterface,
    flat_enums: &[FlatEnum],
) -> Result<Vec<OptionalType>> {
    let mut optionals = component
        .iter_local_types()
        .filter_map(|type_| match type_ {
            Type::Optional { inner_type } => optional_type(inner_type, flat_enums),
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

fn optional_type(inner_type: &Type, flat_enums: &[FlatEnum]) -> Option<OptionalType> {
    let inner = match inner_type {
        Type::Enum { name, .. } => {
            let enum_ = flat_enums.iter().find(|enum_| enum_.rust_name() == name)?;
            OptionalInner {
                rust_name: enum_.rust_name().to_owned(),
                c_name: enum_.c_name().to_owned(),
                c_suffix: enum_.rust_name().to_owned(),
                function_name: enum_.function_name().to_owned(),
                wire_size: OptionalWireSize::Fixed(4),
            }
        }
        type_ => builtin_inner(BuiltinType::from_uniffi_type(type_)?),
    };
    let rust_name = format!("Option<{}>", inner.rust_name);
    let c_name = naming::type_name(&format!("Optional{}", inner.c_suffix));
    let function_name = format!("optional_{}", inner.function_name);

    Some(OptionalType {
        rust_name,
        c_name,
        function_name,
        inner_rust_name: inner.rust_name,
        inner_c_name: inner.c_name,
        inner_function_name: inner.function_name,
        inner_wire_size: inner.wire_size,
    })
}

struct OptionalInner {
    rust_name: String,
    c_name: String,
    c_suffix: String,
    function_name: String,
    wire_size: OptionalWireSize,
}

fn builtin_inner(builtin: BuiltinType) -> OptionalInner {
    let (c_suffix, function_name, wire_size) = match builtin {
        BuiltinType::UInt8 => ("U8", "u8", OptionalWireSize::Fixed(1)),
        BuiltinType::Int8 => ("I8", "i8", OptionalWireSize::Fixed(1)),
        BuiltinType::UInt16 => ("U16", "u16", OptionalWireSize::Fixed(2)),
        BuiltinType::Int16 => ("I16", "i16", OptionalWireSize::Fixed(2)),
        BuiltinType::UInt32 => ("U32", "u32", OptionalWireSize::Fixed(4)),
        BuiltinType::Int32 => ("I32", "i32", OptionalWireSize::Fixed(4)),
        BuiltinType::UInt64 => ("U64", "u64", OptionalWireSize::Fixed(8)),
        BuiltinType::Int64 => ("I64", "i64", OptionalWireSize::Fixed(8)),
        BuiltinType::Boolean => ("Bool", "bool", OptionalWireSize::Fixed(1)),
        BuiltinType::String => ("StringView", "string", OptionalWireSize::LengthPrefixedView),
        BuiltinType::Bytes => ("BytesView", "bytes", OptionalWireSize::LengthPrefixedView),
    };
    OptionalInner {
        rust_name: builtin.rust_name().to_owned(),
        c_name: builtin.c_name().to_owned(),
        c_suffix: c_suffix.to_owned(),
        function_name: function_name.to_owned(),
        wire_size,
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use uniffi_bindgen::ComponentInterface;

    use super::{OptionalWireSize, collect_optional_types};
    use crate::enum_map::collect_flat_enums;

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
        let flat_enums = collect_flat_enums(&component)?;
        let optionals = collect_optional_types(&component, &flat_enums)?;

        assert_eq!(optionals.len(), 3);
        let network = optionals
            .iter()
            .find(|optional| optional.rust_name() == "Option<Network>")
            .context("Option<Network> should be supported")?;
        assert_eq!(network.c_name(), "WalletEngineOptionalNetwork");
        assert_eq!(network.inner_c_name(), "WalletEngineNetwork");
        assert_eq!(network.inner_wire_size(), OptionalWireSize::Fixed(4));
        let string = optionals
            .iter()
            .find(|optional| optional.rust_name() == "Option<String>")
            .context("Option<String> should be supported")?;
        assert_eq!(string.c_name(), "WalletEngineOptionalStringView");
        assert_eq!(
            string.inner_wire_size(),
            OptionalWireSize::LengthPrefixedView
        );
        Ok(())
    }
}
