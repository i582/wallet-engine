use serde::Serialize;
use uniffi_bindgen::ComponentInterface;

pub(super) const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub(super) struct Manifest {
    schema_version: u32,
    components: Vec<ComponentManifest>,
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

impl Manifest {
    pub(super) fn from_components(components: &[ComponentInterface]) -> Self {
        Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            components: components
                .iter()
                .map(ComponentManifest::from_component)
                .collect(),
        }
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
    use uniffi_bindgen::ComponentInterface;

    use super::{MANIFEST_SCHEMA_VERSION, Manifest};

    #[test]
    fn empty_component_still_produces_a_versioned_manifest() {
        let component = ComponentInterface::new("wallet_engine");
        let manifest = Manifest::from_components(&[component]);

        assert_eq!(manifest.schema_version, MANIFEST_SCHEMA_VERSION);
        assert_eq!(manifest.components.len(), 1);
        assert_eq!(manifest.components[0].crate_name, "wallet_engine");
    }
}
