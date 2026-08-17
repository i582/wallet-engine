use std::collections::BTreeSet;

use uniffi_bindgen::{ComponentInterface, interface::Type};

/// Builtin `UniFFI` types supported by the current C generator slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum BuiltinType {
    UInt8,
    Int8,
    UInt16,
    Int16,
    UInt32,
    Int32,
    UInt64,
    Int64,
    Boolean,
    String,
    Bytes,
}

impl BuiltinType {
    pub(super) const fn rust_name(self) -> &'static str {
        match self {
            Self::UInt8 => "u8",
            Self::Int8 => "i8",
            Self::UInt16 => "u16",
            Self::Int16 => "i16",
            Self::UInt32 => "u32",
            Self::Int32 => "i32",
            Self::UInt64 => "u64",
            Self::Int64 => "i64",
            Self::Boolean => "bool",
            Self::String => "String",
            Self::Bytes => "Vec<u8>",
        }
    }

    pub(super) const fn c_name(self) -> &'static str {
        match self {
            Self::UInt8 => "uint8_t",
            Self::Int8 => "int8_t",
            Self::UInt16 => "uint16_t",
            Self::Int16 => "int16_t",
            Self::UInt32 => "uint32_t",
            Self::Int32 => "int32_t",
            Self::UInt64 => "uint64_t",
            Self::Int64 => "int64_t",
            Self::Boolean => "bool",
            Self::String => "WalletEngineStringView",
            Self::Bytes => "WalletEngineBytesView",
        }
    }

    pub(super) const fn from_uniffi_type(type_: &Type) -> Option<Self> {
        match type_ {
            Type::UInt8 => Some(Self::UInt8),
            Type::Int8 => Some(Self::Int8),
            Type::UInt16 => Some(Self::UInt16),
            Type::Int16 => Some(Self::Int16),
            Type::UInt32 => Some(Self::UInt32),
            Type::Int32 => Some(Self::Int32),
            Type::UInt64 => Some(Self::UInt64),
            Type::Int64 => Some(Self::Int64),
            Type::Boolean => Some(Self::Boolean),
            Type::String => Some(Self::String),
            Type::Bytes => Some(Self::Bytes),
            Type::Float32
            | Type::Float64
            | Type::Timestamp
            | Type::Duration
            | Type::Object { .. }
            | Type::Record { .. }
            | Type::Enum { .. }
            | Type::CallbackInterface { .. }
            | Type::Box { .. }
            | Type::Optional { .. }
            | Type::Sequence { .. }
            | Type::Map { .. }
            | Type::Set { .. }
            | Type::Custom { .. } => None,
        }
    }
}

pub(super) fn collect_builtin_types(component: &ComponentInterface) -> Vec<BuiltinType> {
    component
        .iter_local_types()
        .flat_map(Type::iter_types)
        .filter_map(BuiltinType::from_uniffi_type)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use uniffi_bindgen::ComponentInterface;

    use super::{BuiltinType, collect_builtin_types};

    #[test]
    fn collects_nested_builtin_types_once_in_stable_order() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};

            dictionary Example {
                boolean enabled;
                u16 count;
                string name;
                bytes payload;
            };
            ",
            "wallet_engine",
        )?;
        let builtins = collect_builtin_types(&component);

        assert!(builtins.contains(&BuiltinType::UInt16));
        assert!(builtins.contains(&BuiltinType::Boolean));
        assert!(builtins.contains(&BuiltinType::String));
        assert!(builtins.contains(&BuiltinType::Bytes));
        assert!(builtins.windows(2).all(|pair| pair[0] < pair[1]));
        Ok(())
    }
}
