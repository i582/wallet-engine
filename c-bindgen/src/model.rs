use anyhow::{Result, bail};
use serde::Serialize;
use uniffi_bindgen::ComponentInterface;

use crate::type_map::{BuiltinType, collect_builtin_types};

pub(super) const MANIFEST_SCHEMA_VERSION: u32 = 4;
const EXPERIMENTAL_ABI_VERSION: u32 = 0;
const EXPECTED_CRATE_NAME: &str = "wallet_engine";
const EXPECTED_NAMESPACE: &str = "wallet_engine";

#[derive(Debug)]
pub(super) struct BindingsModel {
    abi_version: u32,
    uniffi_contract_version: u32,
    builtin_types: Vec<BuiltinType>,
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
    rendered_semantic_operation_count: usize,
    pending_semantic_operation_count: usize,
}

#[derive(Debug, Serialize)]
struct BuiltinTypeManifest {
    rust: &'static str,
    c: &'static str,
    codec: &'static str,
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

        let builtin_types = collect_builtin_types(component);
        let manifest = Manifest::from_components(components, &builtin_types);
        let private_ffi = PrivateFfi {
            rustbuffer_alloc: component.ffi_rustbuffer_alloc().name().to_owned(),
            rustbuffer_free: component.ffi_rustbuffer_free().name().to_owned(),
        };

        Ok(Self {
            abi_version: EXPERIMENTAL_ABI_VERSION,
            uniffi_contract_version: component.uniffi_contract_version(),
            builtin_types,
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
        self.builtin_types.contains(&builtin)
    }

    pub(super) const fn private_ffi(&self) -> &PrivateFfi {
        &self.private_ffi
    }

    pub(super) const fn manifest(&self) -> &Manifest {
        &self.manifest
    }
}

impl Manifest {
    fn from_components(components: &[ComponentInterface], builtin_types: &[BuiltinType]) -> Self {
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
                phase: "builtin_codecs",
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
            codec: value.codec(),
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
}
