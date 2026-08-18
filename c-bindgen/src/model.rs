use anyhow::{Result, bail};
use serde::Serialize;
use uniffi_bindgen::ComponentInterface;

use crate::{
    enum_map::FlatEnum,
    error_map::{ErrorType, collect_error_types},
    optional_map::{OptionalType, collect_optional_types},
    record_map::{RecordType, collect_record_types},
    sequence_map::{SequenceType, collect_sequence_types},
    type_map::BuiltinType,
    type_registry::TypeRegistry,
};

pub(super) const MANIFEST_SCHEMA_VERSION: u32 = 8;
const EXPERIMENTAL_ABI_VERSION: u32 = 0;
const EXPECTED_CRATE_NAME: &str = "wallet_engine";
const EXPECTED_NAMESPACE: &str = "wallet_engine";

#[derive(Debug)]
pub(super) struct BindingsModel {
    abi_version: u32,
    uniffi_contract_version: u32,
    type_registry: TypeRegistry,
    optional_types: Vec<OptionalType>,
    sequence_types: Vec<SequenceType>,
    record_types: Vec<RecordType>,
    error_types: Vec<ErrorType>,
    private_ffi: PrivateFfi,
    manifest: Manifest,
}

#[derive(Debug)]
pub(super) struct PrivateFfi {
    rustbuffer_alloc: String,
    rustbuffer_free: String,
}

#[derive(Debug, Serialize)]
pub(super) struct Manifest {
    schema_version: u32,
    generation: GenerationManifest,
    components: Vec<ComponentManifest>,
}

#[derive(Debug, Serialize)]
struct GenerationManifest {
    phase: &'static str,
    artifacts: [&'static str; 3],
    rendered_builtin_types: Vec<BuiltinTypeManifest>,
    rendered_flat_enums: Vec<FlatEnumManifest>,
    rendered_optional_types: Vec<OptionalTypeManifest>,
    rendered_sequence_types: Vec<SequenceTypeManifest>,
    rendered_record_types: Vec<RecordTypeManifest>,
    rendered_error_types: Vec<ErrorTypeManifest>,
    rendered_semantic_operation_count: usize,
    pending_semantic_operation_count: usize,
}

#[derive(Debug, Serialize)]
struct BuiltinTypeManifest {
    rust: &'static str,
    c: &'static str,
}

#[derive(Debug, Serialize)]
struct FlatEnumManifest {
    rust: String,
    c: String,
    variants: Vec<FlatEnumVariantManifest>,
}

#[derive(Debug, Serialize)]
struct FlatEnumVariantManifest {
    rust: String,
    c: String,
    value: u32,
}

#[derive(Debug, Serialize)]
struct OptionalTypeManifest {
    rust: String,
    c: String,
    value: OptionalValueManifest,
}

#[derive(Debug, Serialize)]
struct OptionalValueManifest {
    rust: String,
    c: String,
}

#[derive(Debug, Serialize)]
struct SequenceTypeManifest {
    rust: String,
    c: String,
    item: SequenceItemManifest,
}

#[derive(Debug, Serialize)]
struct SequenceItemManifest {
    rust: String,
    c: String,
}

#[derive(Debug, Serialize)]
struct RecordTypeManifest {
    rust: String,
    c: String,
    fields: Vec<RecordFieldManifest>,
}

#[derive(Debug, Serialize)]
struct RecordFieldManifest {
    rust: String,
    c: String,
    rust_type: String,
    c_type: String,
}

#[derive(Debug, Serialize)]
struct ErrorTypeManifest {
    rust: String,
    c: String,
    tag_c: String,
    variants: Vec<ErrorVariantManifest>,
}

#[derive(Debug, Serialize)]
struct ErrorVariantManifest {
    rust: String,
    c: String,
    value: u32,
    payload_c: Option<String>,
    fields: Vec<ErrorFieldManifest>,
}

#[derive(Debug, Serialize)]
struct ErrorFieldManifest {
    rust: String,
    c: String,
    rust_type: String,
    c_type: String,
}

#[derive(Debug, Serialize)]
struct ComponentManifest {
    crate_name: String,
    namespace: String,
    uniffi_contract_version: u32,
    type_count: usize,
    record_count: usize,
    enum_count: usize,
    rust_object_count: usize,
    foreign_interface_count: usize,
    rust_callable_count: usize,
    async_rust_callable_count: usize,
    foreign_method_count: usize,
    semantic_operation_count: usize,
    ffi_function_count: usize,
    checksum_count: usize,
}

impl BindingsModel {
    pub(super) fn from_components(components: &[ComponentInterface]) -> Result<Self> {
        let [component] = components else {
            bail!(
                "C bindings require exactly one UniFFI component, found {}",
                components.len()
            );
        };
        if component.crate_name() != EXPECTED_CRATE_NAME
            || (!component.namespace().is_empty() && component.namespace() != EXPECTED_NAMESPACE)
        {
            bail!(
                "expected UniFFI component {EXPECTED_CRATE_NAME}/{EXPECTED_NAMESPACE}, found {}/{}",
                component.crate_name(),
                component.namespace()
            );
        }

        let mut type_registry = TypeRegistry::collect(component)?;
        let optional_types = collect_optional_types(component, &type_registry)?;
        let sequence_types = collect_sequence_types(component, &type_registry)?;
        for optional in &optional_types {
            type_registry.register_type(optional.uniffi_type(), optional.registered_type())?;
        }
        for sequence in &sequence_types {
            type_registry.register_type(sequence.uniffi_type(), sequence.registered_type())?;
        }
        let record_types = collect_record_types(component, &mut type_registry)?;
        let error_types = collect_error_types(component, &mut type_registry)?;
        let manifest = Manifest::from_components(
            components,
            type_registry.builtin_types(),
            type_registry.flat_enums(),
            &optional_types,
            &sequence_types,
            &record_types,
            &error_types,
        );
        let private_ffi = PrivateFfi {
            rustbuffer_alloc: component.ffi_rustbuffer_alloc().name().to_owned(),
            rustbuffer_free: component.ffi_rustbuffer_free().name().to_owned(),
        };

        Ok(Self {
            abi_version: EXPERIMENTAL_ABI_VERSION,
            uniffi_contract_version: component.uniffi_contract_version(),
            type_registry,
            optional_types,
            sequence_types,
            record_types,
            error_types,
            private_ffi,
            manifest,
        })
    }

    pub(super) const fn abi_version(&self) -> u32 {
        self.abi_version
    }

    pub(super) const fn uniffi_contract_version(&self) -> u32 {
        self.uniffi_contract_version
    }

    pub(super) fn has_builtin_type(&self, builtin: BuiltinType) -> bool {
        self.type_registry.has_builtin_type(builtin)
    }

    /// `Option<T>` uses the private `u8` codec for its `UniFFI` 0/1 presence tag,
    /// even when no public API value has type `u8` or `i8`.
    pub(super) fn needs_u8_wire_codec(&self) -> bool {
        self.has_builtin_type(BuiltinType::UInt8)
            || self.has_builtin_type(BuiltinType::Int8)
            || self.has_optional_types()
    }

    /// `Sequence<T>` stores its item count as a signed big-endian `i32`.
    pub(super) fn needs_i32_wire_codec(&self) -> bool {
        self.has_builtin_type(BuiltinType::Int32)
            || self.has_flat_enums()
            || self.has_sequence_types()
            || self.has_error_types()
    }

    pub(super) fn has_wire_types(&self) -> bool {
        !self.type_registry.is_empty()
            || !self.optional_types.is_empty()
            || !self.sequence_types.is_empty()
            || !self.record_types.is_empty()
            || !self.error_types.is_empty()
    }

    pub(super) const fn has_flat_enums(&self) -> bool {
        self.type_registry.has_flat_enums()
    }

    pub(super) fn needs_rustbuffer_runtime(&self) -> bool {
        self.has_builtin_type(BuiltinType::String)
            || self.has_builtin_type(BuiltinType::Bytes)
            || self.has_flat_enums()
            || self.has_optional_types()
            || self.has_sequence_types()
            || self.has_record_types()
            || self.has_error_types()
    }

    pub(super) fn needs_output_arena(&self) -> bool {
        self.has_sequence_types()
            || self.has_record_types()
            || self.error_types.iter().any(ErrorType::read_needs_arena)
    }

    pub(super) fn flat_enums(&self) -> &[FlatEnum] {
        self.type_registry.flat_enums()
    }

    pub(super) const fn has_optional_types(&self) -> bool {
        !self.optional_types.is_empty()
    }

    pub(super) fn optional_types(&self) -> &[OptionalType] {
        &self.optional_types
    }

    pub(super) const fn has_sequence_types(&self) -> bool {
        !self.sequence_types.is_empty()
    }

    pub(super) fn sequence_types(&self) -> &[SequenceType] {
        &self.sequence_types
    }

    pub(super) const fn has_record_types(&self) -> bool {
        !self.record_types.is_empty()
    }

    pub(super) fn record_types(&self) -> &[RecordType] {
        &self.record_types
    }

    pub(super) const fn has_error_types(&self) -> bool {
        !self.error_types.is_empty()
    }

    pub(super) fn error_types(&self) -> &[ErrorType] {
        &self.error_types
    }

    pub(super) const fn private_ffi(&self) -> &PrivateFfi {
        &self.private_ffi
    }

    pub(super) const fn manifest(&self) -> &Manifest {
        &self.manifest
    }
}

impl Manifest {
    fn from_components(
        components: &[ComponentInterface],
        builtin_types: &[BuiltinType],
        flat_enums: &[FlatEnum],
        optional_types: &[OptionalType],
        sequence_types: &[SequenceType],
        record_types: &[RecordType],
        error_types: &[ErrorType],
    ) -> Self {
        let component_manifests = components
            .iter()
            .map(ComponentManifest::from_component)
            .collect::<Vec<_>>();
        let pending_semantic_operation_count = component_manifests
            .iter()
            .map(|component| component.semantic_operation_count)
            .sum();

        Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            generation: GenerationManifest {
                phase: "rich-errors",
                artifacts: [
                    "wallet_engine.h",
                    "wallet_engine.c",
                    "wallet_engine.c-api.json",
                ],
                rendered_builtin_types: builtin_types
                    .iter()
                    .copied()
                    .map(BuiltinTypeManifest::from)
                    .collect(),
                rendered_flat_enums: flat_enums.iter().map(FlatEnumManifest::from).collect(),
                rendered_optional_types: optional_types
                    .iter()
                    .map(OptionalTypeManifest::from)
                    .collect(),
                rendered_sequence_types: sequence_types
                    .iter()
                    .map(SequenceTypeManifest::from)
                    .collect(),
                rendered_record_types: record_types.iter().map(RecordTypeManifest::from).collect(),
                rendered_error_types: error_types.iter().map(ErrorTypeManifest::from).collect(),
                rendered_semantic_operation_count: 0,
                pending_semantic_operation_count,
            },
            components: component_manifests,
        }
    }
}

impl From<BuiltinType> for BuiltinTypeManifest {
    fn from(value: BuiltinType) -> Self {
        Self {
            rust: value.rust_name(),
            c: value.c_name(),
        }
    }
}

impl From<&FlatEnum> for FlatEnumManifest {
    fn from(value: &FlatEnum) -> Self {
        Self {
            rust: value.rust_name().to_owned(),
            c: value.c_name().to_owned(),
            variants: value
                .variants()
                .iter()
                .map(FlatEnumVariantManifest::from)
                .collect(),
        }
    }
}

impl From<&crate::enum_map::FlatEnumVariant> for FlatEnumVariantManifest {
    fn from(value: &crate::enum_map::FlatEnumVariant) -> Self {
        Self {
            rust: value.rust_name().to_owned(),
            c: value.c_constant().to_owned(),
            value: value.public_value(),
        }
    }
}

impl From<&OptionalType> for OptionalTypeManifest {
    fn from(value: &OptionalType) -> Self {
        Self {
            rust: value.rust_name().to_owned(),
            c: value.c_name().to_owned(),
            value: OptionalValueManifest {
                rust: value.inner_rust_name().to_owned(),
                c: value.inner_c_name().to_owned(),
            },
        }
    }
}

impl From<&SequenceType> for SequenceTypeManifest {
    fn from(value: &SequenceType) -> Self {
        Self {
            rust: value.rust_name().to_owned(),
            c: value.c_name().to_owned(),
            item: SequenceItemManifest {
                rust: value.inner_rust_name().to_owned(),
                c: value.inner_c_name().to_owned(),
            },
        }
    }
}

impl From<&RecordType> for RecordTypeManifest {
    fn from(value: &RecordType) -> Self {
        Self {
            rust: value.rust_name().to_owned(),
            c: value.c_name().to_owned(),
            fields: value
                .fields()
                .iter()
                .map(|field| RecordFieldManifest {
                    rust: field.rust_name().to_owned(),
                    c: field.c_name().to_owned(),
                    rust_type: field.rust_type_name().to_owned(),
                    c_type: field.c_type_name().to_owned(),
                })
                .collect(),
        }
    }
}

impl From<&ErrorType> for ErrorTypeManifest {
    fn from(value: &ErrorType) -> Self {
        Self {
            rust: value.rust_name().to_owned(),
            c: value.c_name().to_owned(),
            tag_c: value.tag_c_name().to_owned(),
            variants: value
                .variants()
                .iter()
                .map(|variant| ErrorVariantManifest {
                    rust: variant.rust_name().to_owned(),
                    c: variant.c_constant().to_owned(),
                    value: variant.public_value(),
                    payload_c: variant.payload_c_name().map(str::to_owned),
                    fields: variant
                        .fields()
                        .iter()
                        .map(|field| ErrorFieldManifest {
                            rust: field.rust_name().to_owned(),
                            c: field.c_name().to_owned(),
                            rust_type: field.rust_type_name().to_owned(),
                            c_type: field.c_type_name().to_owned(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

impl PrivateFfi {
    pub(super) fn rustbuffer_alloc(&self) -> &str {
        &self.rustbuffer_alloc
    }

    pub(super) fn rustbuffer_free(&self) -> &str {
        &self.rustbuffer_free
    }
}

impl ComponentManifest {
    fn from_component(component: &ComponentInterface) -> Self {
        let foreign_object_method_count = component
            .object_definitions()
            .iter()
            .filter(|object| object.has_callback_interface())
            .map(|object| object.methods().len())
            .sum::<usize>();
        let async_foreign_object_method_count = component
            .object_definitions()
            .iter()
            .filter(|object| object.has_callback_interface())
            .flat_map(|object| object.methods())
            .filter(|method| method.is_async())
            .count();
        let callback_method_count = component
            .callback_interface_definitions()
            .iter()
            .map(|interface| interface.methods().len())
            .sum::<usize>();
        let all_callable_count = component.iter_callables().count();
        let all_async_callable_count = component
            .iter_callables()
            .filter(|callable| callable.is_async())
            .count();
        let rust_callable_count = all_callable_count.saturating_sub(foreign_object_method_count);
        let async_rust_callable_count =
            all_async_callable_count.saturating_sub(async_foreign_object_method_count);
        let foreign_method_count =
            foreign_object_method_count.saturating_add(callback_method_count);

        Self {
            crate_name: component.crate_name().to_owned(),
            namespace: component.namespace().to_owned(),
            uniffi_contract_version: component.uniffi_contract_version(),
            type_count: component.iter_local_types().count(),
            record_count: component.record_definitions().len(),
            enum_count: component.enum_definitions().len(),
            rust_object_count: component
                .object_definitions()
                .iter()
                .filter(|object| !object.has_callback_interface())
                .count(),
            foreign_interface_count: component
                .object_definitions()
                .iter()
                .filter(|object| object.has_callback_interface())
                .count()
                .saturating_add(component.callback_interface_definitions().len()),
            rust_callable_count,
            async_rust_callable_count,
            foreign_method_count,
            semantic_operation_count: rust_callable_count.saturating_add(foreign_method_count),
            ffi_function_count: component.iter_ffi_function_definitions().count(),
            checksum_count: component.iter_checksums().count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use uniffi_bindgen::ComponentInterface;

    use super::{BindingsModel, MANIFEST_SCHEMA_VERSION};

    #[test]
    fn empty_component_still_produces_a_versioned_manifest() -> Result<()> {
        let component = ComponentInterface::new("wallet_engine");
        let model = BindingsModel::from_components(&[component])?;
        let manifest = model.manifest();

        assert_eq!(manifest.schema_version, MANIFEST_SCHEMA_VERSION);
        assert_eq!(manifest.generation.rendered_semantic_operation_count, 0);
        assert!(manifest.generation.rendered_builtin_types.is_empty());
        assert!(manifest.generation.rendered_flat_enums.is_empty());
        assert!(manifest.generation.rendered_optional_types.is_empty());
        assert!(manifest.generation.rendered_sequence_types.is_empty());
        assert!(manifest.generation.rendered_record_types.is_empty());
        assert!(manifest.generation.rendered_error_types.is_empty());
        assert_eq!(manifest.components.len(), 1);
        assert_eq!(manifest.components[0].crate_name, "wallet_engine");
        Ok(())
    }

    #[test]
    fn rejects_more_than_one_component() {
        let components = [
            ComponentInterface::new("wallet_engine"),
            ComponentInterface::new("another_component"),
        ];

        assert!(BindingsModel::from_components(&components).is_err());
    }

    #[test]
    fn manifest_records_public_flat_enum_values_without_private_wire_tags() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r#"
            namespace wallet_engine {};
            enum Network { "mainnet", "testnet" };
            "#,
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let manifest = model.manifest();
        let enum_ = &manifest.generation.rendered_flat_enums[0];

        assert_eq!(manifest.generation.phase, "rich-errors");
        assert_eq!(enum_.rust, "Network");
        assert_eq!(enum_.variants[0].value, 0);
        assert_eq!(enum_.variants[1].value, 1);
        assert!(!serde_json::to_string(manifest)?.contains("wire_tag"));
        Ok(())
    }

    #[test]
    fn manifest_records_public_optional_shape() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            dictionary Example { u64? revision; };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let optional = &model.manifest().generation.rendered_optional_types[0];

        assert_eq!(optional.rust, "Option<u64>");
        assert_eq!(optional.c, "WalletEngineOptionalU64");
        assert_eq!(optional.value.rust, "u64");
        assert_eq!(optional.value.c, "uint64_t");
        Ok(())
    }

    #[test]
    fn manifest_records_public_sequence_shape() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            dictionary Example { sequence<string> names; };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let sequence = &model.manifest().generation.rendered_sequence_types[0];

        assert_eq!(sequence.rust, "Vec<String>");
        assert_eq!(sequence.c, "WalletEngineStringListView");
        assert_eq!(sequence.item.rust, "String");
        assert_eq!(sequence.item.c, "WalletEngineStringView");
        Ok(())
    }

    #[test]
    fn manifest_records_public_record_fields() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};
            dictionary Example { string label; u64 revision; };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let record = &model.manifest().generation.rendered_record_types[0];

        assert_eq!(record.rust, "Example");
        assert_eq!(record.c, "WalletEngineExampleView");
        assert_eq!(record.fields[0].rust, "label");
        assert_eq!(record.fields[0].c_type, "WalletEngineStringView");
        assert_eq!(record.fields[1].rust_type, "u64");
        assert_eq!(record.fields[1].c_type, "uint64_t");
        Ok(())
    }

    #[test]
    fn manifest_records_public_error_variants_without_private_wire_tags() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine { [Throws=HostFailure] void call_host(); };
            [Error]
            interface HostFailure {
                Cancelled();
                Failed(u64 code, string diagnostic);
            };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let error = &model.manifest().generation.rendered_error_types[0];

        assert_eq!(error.rust, "HostFailure");
        assert_eq!(error.c, "WalletEngineHostFailure");
        assert_eq!(error.tag_c, "WalletEngineHostFailureTag");
        assert_eq!(error.variants[0].value, 0);
        assert!(error.variants[0].payload_c.is_none());
        assert_eq!(error.variants[1].value, 1);
        assert_eq!(
            error.variants[1].payload_c.as_deref(),
            Some("WalletEngineHostFailureFailedPayload")
        );
        assert_eq!(error.variants[1].fields[0].c_type, "uint64_t");
        assert!(!serde_json::to_string(model.manifest())?.contains("wire_tag"));
        Ok(())
    }
}
