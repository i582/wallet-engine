use std::collections::BTreeSet;

use anyhow::{Context, Result, ensure};
use uniffi_bindgen::{
    ComponentInterface,
    interface::{AsType, Enum, Type, Variant},
};

use crate::{
    naming,
    type_registry::{NestedWireSize, RegisteredType, TypeRegistry},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ErrorType {
    uniffi_type: Type,
    rust_name: String,
    c_name: String,
    tag_c_name: String,
    payload_c_name: String,
    function_name: String,
    variants: Vec<ErrorVariant>,
    read_needs_arena: bool,
}

impl ErrorType {
    pub(super) fn rust_name(&self) -> &str {
        &self.rust_name
    }

    pub(super) fn c_name(&self) -> &str {
        &self.c_name
    }

    pub(super) fn tag_c_name(&self) -> &str {
        &self.tag_c_name
    }

    pub(super) fn payload_c_name(&self) -> &str {
        &self.payload_c_name
    }

    pub(super) fn function_name(&self) -> &str {
        &self.function_name
    }

    pub(super) fn variants(&self) -> &[ErrorVariant] {
        &self.variants
    }

    pub(super) const fn read_needs_arena(&self) -> bool {
        self.read_needs_arena
    }

    fn registered_type(&self) -> RegisteredType {
        RegisteredType::compound(
            self.rust_name.clone(),
            self.c_name.clone(),
            self.rust_name.clone(),
            self.function_name.clone(),
            4,
            self.read_needs_arena,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ErrorVariant {
    rust_name: String,
    c_constant: String,
    public_value: u32,
    wire_tag: i32,
    payload_c_name: Option<String>,
    payload_member_name: String,
    fields: Vec<ErrorField>,
}

impl ErrorVariant {
    pub(super) fn rust_name(&self) -> &str {
        &self.rust_name
    }

    pub(super) fn c_constant(&self) -> &str {
        &self.c_constant
    }

    pub(super) const fn public_value(&self) -> u32 {
        self.public_value
    }

    pub(super) const fn wire_tag(&self) -> i32 {
        self.wire_tag
    }

    pub(super) fn payload_c_name(&self) -> Option<&str> {
        self.payload_c_name.as_deref()
    }

    pub(super) fn payload_member_name(&self) -> &str {
        &self.payload_member_name
    }

    pub(super) fn fields(&self) -> &[ErrorField] {
        &self.fields
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ErrorField {
    rust_name: String,
    c_name: String,
    rust_type_name: String,
    c_type_name: String,
    codec_name: String,
    nested_wire_size: NestedWireSize,
    read_needs_arena: bool,
}

impl ErrorField {
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
}

pub(super) fn collect_error_types(
    component: &ComponentInterface,
    types: &mut TypeRegistry,
) -> Result<Vec<ErrorType>> {
    let mut remaining = component
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
        .collect::<Vec<_>>();
    remaining.sort_by(|left, right| left.name().cmp(right.name()));

    let mut errors = Vec::new();
    loop {
        let previous_len = remaining.len();
        let mut pending = Vec::new();
        for enum_ in remaining {
            let Some(error) = error_type(enum_, types)? else {
                pending.push(enum_);
                continue;
            };
            reserve_auxiliary_c_names(&error, types)?;
            types.register_type(&error.uniffi_type, error.registered_type())?;
            errors.push(error);
        }
        if pending.is_empty() || pending.len() == previous_len {
            break;
        }
        remaining = pending;
    }
    Ok(errors)
}

fn error_type(enum_: &Enum, types: &TypeRegistry) -> Result<Option<ErrorType>> {
    let rust_name = enum_.name().to_owned();
    let mut constants = BTreeSet::new();
    let mut payload_members = BTreeSet::new();
    let variants = enum_
        .variants()
        .iter()
        .enumerate()
        .map(|(variant_index, variant)| {
            error_variant(
                &rust_name,
                variant_index,
                variant,
                types,
                &mut constants,
                &mut payload_members,
            )
        })
        .collect::<Result<Option<Vec<_>>>>()?;
    let Some(variants) = variants else {
        return Ok(None);
    };
    ensure!(!variants.is_empty(), "error {rust_name} has no variants");
    ensure!(
        variants.iter().any(|variant| !variant.fields.is_empty()),
        "rich error {rust_name} has no payload variants"
    );
    let read_needs_arena = variants
        .iter()
        .flat_map(|variant| &variant.fields)
        .any(ErrorField::read_needs_arena);

    Ok(Some(ErrorType {
        uniffi_type: enum_.as_type(),
        c_name: naming::type_name(&rust_name),
        tag_c_name: naming::type_name(&format!("{rust_name}Tag")),
        payload_c_name: naming::type_name(&format!("{rust_name}Payload")),
        function_name: naming::function_name(&rust_name),
        rust_name,
        variants,
        read_needs_arena,
    }))
}

fn error_variant(
    error_rust_name: &str,
    variant_index: usize,
    variant: &Variant,
    types: &TypeRegistry,
    constants: &mut BTreeSet<String>,
    payload_members: &mut BTreeSet<String>,
) -> Result<Option<ErrorVariant>> {
    let public_value = u32::try_from(variant_index).with_context(|| {
        format!("error {error_rust_name} has too many variants for the public C ABI")
    })?;
    let wire_tag = i32::try_from(variant_index.saturating_add(1)).with_context(|| {
        format!("error {error_rust_name} has too many variants for the UniFFI wire ABI")
    })?;
    let c_constant = naming::constant_name(error_rust_name, variant.name());
    ensure!(
        constants.insert(c_constant.clone()),
        "error {error_rust_name} has C constant collision at {c_constant}"
    );

    let mut c_field_names = BTreeSet::new();
    let fields = variant
        .fields()
        .iter()
        .enumerate()
        .map(|(field_index, field)| {
            let field_type = field.as_type();
            let registered = types.resolve(&field_type)?;
            let rust_field_name = field.name().to_owned();
            let c_name = if rust_field_name.is_empty() {
                format!("field_{field_index}")
            } else {
                naming::field_name(&rust_field_name)
            };
            Some((registered, rust_field_name, c_name))
        })
        .collect::<Option<Vec<_>>>();
    let Some(fields) = fields else {
        return Ok(None);
    };
    let fields = fields
        .into_iter()
        .map(|(registered, rust_field_name, c_name)| {
            ensure!(
                c_field_names.insert(c_name.clone()),
                "error {error_rust_name} variant {} produces duplicate C field {c_name}",
                variant.name()
            );
            Ok(ErrorField {
                rust_name: rust_field_name,
                c_name,
                rust_type_name: registered.rust_name().to_owned(),
                c_type_name: registered.c_name().to_owned(),
                codec_name: registered.codec_name().to_owned(),
                nested_wire_size: registered.nested_wire_size(),
                read_needs_arena: registered.read_needs_arena(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let payload_member_name = naming::field_name(variant.name());
    let payload_c_name = if fields.is_empty() {
        None
    } else {
        ensure!(
            payload_members.insert(payload_member_name.clone()),
            "error {error_rust_name} produces duplicate C payload member {payload_member_name}"
        );
        Some(naming::type_name(&format!(
            "{error_rust_name}{}Payload",
            variant.name()
        )))
    };

    Ok(Some(ErrorVariant {
        rust_name: variant.name().to_owned(),
        c_constant,
        public_value,
        wire_tag,
        payload_c_name,
        payload_member_name,
        fields,
    }))
}

fn reserve_auxiliary_c_names(error: &ErrorType, types: &mut TypeRegistry) -> Result<()> {
    types.reserve_c_name(error.tag_c_name())?;
    types.reserve_c_name(error.payload_c_name())?;
    for variant in error.variants() {
        if let Some(payload_c_name) = variant.payload_c_name() {
            types.reserve_c_name(payload_c_name)?;
        }
    }
    Ok(())
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
