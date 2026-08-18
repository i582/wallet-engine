use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use uniffi_bindgen::{ComponentInterface, interface::Type};

use crate::{
    enum_map::{FlatEnum, collect_flat_enums},
    type_map::{BuiltinType, collect_builtin_types},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NestedWireSize {
    Fixed(usize),
    LengthPrefixedView,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RegisteredType {
    rust_name: String,
    c_name: String,
    c_type_label: String,
    codec_name: String,
    nested_wire_size: NestedWireSize,
    minimum_wire_size: usize,
    read_needs_arena: bool,
}

impl RegisteredType {
    pub(super) fn rust_name(&self) -> &str {
        &self.rust_name
    }

    pub(super) fn c_name(&self) -> &str {
        &self.c_name
    }

    pub(super) fn c_type_label(&self) -> &str {
        &self.c_type_label
    }

    pub(super) fn codec_name(&self) -> &str {
        &self.codec_name
    }

    pub(super) const fn nested_wire_size(&self) -> NestedWireSize {
        self.nested_wire_size
    }

    pub(super) const fn minimum_wire_size(&self) -> usize {
        self.minimum_wire_size
    }

    pub(super) const fn read_needs_arena(&self) -> bool {
        self.read_needs_arena
    }

    pub(super) const fn compound(
        rust_name: String,
        c_name: String,
        c_type_label: String,
        codec_name: String,
        minimum_wire_size: usize,
        read_needs_arena: bool,
    ) -> Self {
        Self {
            rust_name,
            c_name,
            c_type_label,
            codec_name,
            nested_wire_size: NestedWireSize::Dynamic,
            minimum_wire_size,
            read_needs_arena,
        }
    }

    fn from_builtin(builtin: BuiltinType) -> Self {
        let (c_type_label, codec_name, nested_wire_size, minimum_wire_size) = match builtin {
            BuiltinType::UInt8 => ("U8", "u8", NestedWireSize::Fixed(1), 1),
            BuiltinType::Int8 => ("I8", "i8", NestedWireSize::Fixed(1), 1),
            BuiltinType::UInt16 => ("U16", "u16", NestedWireSize::Fixed(2), 2),
            BuiltinType::Int16 => ("I16", "i16", NestedWireSize::Fixed(2), 2),
            BuiltinType::UInt32 => ("U32", "u32", NestedWireSize::Fixed(4), 4),
            BuiltinType::Int32 => ("I32", "i32", NestedWireSize::Fixed(4), 4),
            BuiltinType::UInt64 => ("U64", "u64", NestedWireSize::Fixed(8), 8),
            BuiltinType::Int64 => ("I64", "i64", NestedWireSize::Fixed(8), 8),
            BuiltinType::Boolean => ("Bool", "bool", NestedWireSize::Fixed(1), 1),
            BuiltinType::String => (
                "StringView",
                "string",
                NestedWireSize::LengthPrefixedView,
                4,
            ),
            BuiltinType::Bytes => ("BytesView", "bytes", NestedWireSize::LengthPrefixedView, 4),
        };
        Self {
            rust_name: builtin.rust_name().to_owned(),
            c_name: builtin.c_name().to_owned(),
            c_type_label: c_type_label.to_owned(),
            codec_name: codec_name.to_owned(),
            nested_wire_size,
            minimum_wire_size,
            read_needs_arena: false,
        }
    }

    fn from_flat_enum(enum_: &FlatEnum) -> Self {
        Self {
            rust_name: enum_.rust_name().to_owned(),
            c_name: enum_.c_name().to_owned(),
            c_type_label: enum_.rust_name().to_owned(),
            codec_name: enum_.function_name().to_owned(),
            nested_wire_size: NestedWireSize::Fixed(4),
            minimum_wire_size: 4,
            read_needs_arena: false,
        }
    }
}

#[derive(Debug)]
pub(super) struct TypeRegistry {
    crate_name: String,
    builtin_types: Vec<BuiltinType>,
    flat_enums: Vec<FlatEnum>,
    types: BTreeMap<TypeKey, RegisteredType>,
    c_names: BTreeSet<String>,
    codec_names: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum TypeKey {
    Builtin(BuiltinType),
    Enum(String),
    Optional(Box<Self>),
    Sequence(Box<Self>),
    Record(String),
}

impl TypeRegistry {
    pub(super) fn collect(component: &ComponentInterface) -> Result<Self> {
        let builtin_types = collect_builtin_types(component);
        let flat_enums = collect_flat_enums(component)?;
        let mut entries = builtin_types
            .iter()
            .copied()
            .map(|builtin| {
                (
                    TypeKey::Builtin(builtin),
                    RegisteredType::from_builtin(builtin),
                )
            })
            .collect::<Vec<_>>();
        for enum_ in &flat_enums {
            entries.push((
                TypeKey::Enum(enum_.rust_name().to_owned()),
                RegisteredType::from_flat_enum(enum_),
            ));
        }

        let mut registry = Self {
            crate_name: component.crate_name().to_owned(),
            builtin_types,
            flat_enums,
            types: BTreeMap::new(),
            c_names: BTreeSet::new(),
            codec_names: BTreeSet::new(),
        };
        for (key, registered) in entries {
            registry.register(key, registered)?;
        }
        Ok(registry)
    }

    pub(super) fn resolve(&self, type_: &Type) -> Option<&RegisteredType> {
        self.key_for(type_).and_then(|key| self.types.get(&key))
    }

    pub(super) fn register_type(&mut self, type_: &Type, registered: RegisteredType) -> Result<()> {
        let Some(key) = self.key_for(type_) else {
            anyhow::bail!("cannot register unsupported or external UniFFI type {type_:?}");
        };
        self.register(key, registered)
    }

    pub(super) fn reserve_c_name(&mut self, c_name: &str) -> Result<()> {
        ensure!(
            !self.c_names.contains(c_name),
            "C type {c_name} is already registered"
        );
        self.c_names.insert(c_name.to_owned());
        Ok(())
    }

    pub(super) fn has_builtin_type(&self, builtin: BuiltinType) -> bool {
        self.builtin_types.contains(&builtin)
    }

    pub(super) fn builtin_types(&self) -> &[BuiltinType] {
        &self.builtin_types
    }

    pub(super) fn flat_enums(&self) -> &[FlatEnum] {
        &self.flat_enums
    }

    pub(super) const fn has_flat_enums(&self) -> bool {
        !self.flat_enums.is_empty()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    fn key_for(&self, type_: &Type) -> Option<TypeKey> {
        if let Some(builtin) = BuiltinType::from_uniffi_type(type_) {
            return Some(TypeKey::Builtin(builtin));
        }
        match type_ {
            Type::Enum { module_path, name }
                if module_path.split("::").next() == Some(self.crate_name.as_str()) =>
            {
                Some(TypeKey::Enum(name.clone()))
            }
            Type::Optional { inner_type } => self
                .key_for(inner_type)
                .map(|inner| TypeKey::Optional(Box::new(inner))),
            Type::Sequence { inner_type } => self
                .key_for(inner_type)
                .map(|inner| TypeKey::Sequence(Box::new(inner))),
            Type::Record { module_path, name }
                if module_path.split("::").next() == Some(self.crate_name.as_str()) =>
            {
                Some(TypeKey::Record(name.clone()))
            }
            _ => None,
        }
    }

    fn register(&mut self, key: TypeKey, registered: RegisteredType) -> Result<()> {
        ensure!(
            !self.types.contains_key(&key),
            "UniFFI type {key:?} is already registered"
        );
        ensure!(
            !self.c_names.contains(registered.c_name()),
            "C type {} is already registered",
            registered.c_name()
        );
        ensure!(
            !self.codec_names.contains(registered.codec_name()),
            "codec {} is already registered",
            registered.codec_name()
        );
        self.c_names.insert(registered.c_name.clone());
        self.codec_names.insert(registered.codec_name.clone());
        self.types.insert(key, registered);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use anyhow::{Context, Result};
    use uniffi_bindgen::{ComponentInterface, interface::Type};

    use super::{NestedWireSize, RegisteredType, TypeKey, TypeRegistry};
    use crate::type_map::BuiltinType;

    #[test]
    fn resolves_builtins_and_supported_flat_enums() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r#"
            namespace wallet_engine {};
            dictionary Example { u64 revision; Network network; Payload payload; };
            enum Network { "mainnet", "testnet" };
            [Enum]
            interface Payload { Value(u64 value); };
            "#,
            "wallet_engine",
        )?;
        let registry = TypeRegistry::collect(&component)?;
        let integer = registry
            .resolve(&Type::UInt64)
            .context("u64 should be registered")?;
        assert_eq!(integer.c_name(), "uint64_t");
        assert_eq!(integer.codec_name(), "u64");

        let network_type = Type::Enum {
            module_path: "wallet_engine".to_owned(),
            name: "Network".to_owned(),
        };
        let network = registry
            .resolve(&network_type)
            .context("Network should be registered")?;
        assert_eq!(network.c_name(), "WalletEngineNetwork");
        assert_eq!(network.nested_wire_size(), NestedWireSize::Fixed(4));

        let payload_type = Type::Enum {
            module_path: "wallet_engine".to_owned(),
            name: "Payload".to_owned(),
        };
        assert!(registry.resolve(&payload_type).is_none());
        let external_network = Type::Enum {
            module_path: "another_crate".to_owned(),
            name: "Network".to_owned(),
        };
        assert!(registry.resolve(&external_network).is_none());
        Ok(())
    }

    #[test]
    fn rejects_public_c_name_collisions() -> Result<()> {
        let mut registry = TypeRegistry {
            crate_name: "wallet_engine".to_owned(),
            builtin_types: Vec::new(),
            flat_enums: Vec::new(),
            types: BTreeMap::new(),
            c_names: BTreeSet::new(),
            codec_names: BTreeSet::new(),
        };
        let first = RegisteredType {
            rust_name: "First".to_owned(),
            c_name: "WalletEngineSame".to_owned(),
            c_type_label: "First".to_owned(),
            codec_name: "first".to_owned(),
            nested_wire_size: NestedWireSize::Fixed(1),
            minimum_wire_size: 1,
            read_needs_arena: false,
        };
        let second = RegisteredType {
            rust_name: "Second".to_owned(),
            c_name: "WalletEngineSame".to_owned(),
            c_type_label: "Second".to_owned(),
            codec_name: "second".to_owned(),
            nested_wire_size: NestedWireSize::Fixed(1),
            minimum_wire_size: 1,
            read_needs_arena: false,
        };

        registry.register(TypeKey::Builtin(BuiltinType::UInt8), first)?;
        assert!(
            registry
                .register(TypeKey::Builtin(BuiltinType::UInt16), second)
                .is_err()
        );
        Ok(())
    }
}
