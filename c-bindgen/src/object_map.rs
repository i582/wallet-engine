use anyhow::Result;
use uniffi_bindgen::{
    ComponentInterface,
    interface::{AsType, Type},
};

use crate::{naming, type_registry::TypeRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HandleKind {
    RustObject,
    ForeignCallbackInterface,
}

impl HandleKind {
    pub(super) const fn manifest_name(self) -> &'static str {
        match self {
            Self::RustObject => "rust_object",
            Self::ForeignCallbackInterface => "foreign_callback_interface",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObjectHandle {
    rust_name: String,
    c_name: String,
    function_name: String,
    kind: HandleKind,
    clone_symbol: Option<String>,
    free_symbol: Option<String>,
}

impl ObjectHandle {
    pub(super) fn rust_name(&self) -> &str {
        &self.rust_name
    }

    pub(super) fn c_name(&self) -> &str {
        &self.c_name
    }

    pub(super) fn function_name(&self) -> &str {
        &self.function_name
    }

    pub(super) const fn kind(&self) -> HandleKind {
        self.kind
    }

    pub(super) fn clone_symbol(&self) -> Option<&str> {
        self.clone_symbol.as_deref()
    }

    pub(super) fn free_symbol(&self) -> Option<&str> {
        self.free_symbol.as_deref()
    }
}

pub(super) fn collect_object_handles(
    component: &ComponentInterface,
    types: &mut TypeRegistry,
) -> Result<Vec<ObjectHandle>> {
    let mut handles = component
        .object_definitions()
        .iter()
        .filter(|object| !object.remote() && is_local(&object.as_type(), component.crate_name()))
        .map(|object| {
            let kind = if object.has_callback_interface() {
                HandleKind::ForeignCallbackInterface
            } else {
                HandleKind::RustObject
            };
            ObjectHandle {
                rust_name: object.name().to_owned(),
                c_name: naming::type_name(object.name()),
                function_name: naming::function_name(object.name()),
                kind,
                clone_symbol: (kind == HandleKind::RustObject)
                    .then(|| object.ffi_object_clone().name().to_owned()),
                free_symbol: (kind == HandleKind::RustObject)
                    .then(|| object.ffi_object_free().name().to_owned()),
            }
        })
        .chain(
            component
                .callback_interface_definitions()
                .iter()
                .filter(|interface| is_local(&interface.as_type(), component.crate_name()))
                .map(|interface| ObjectHandle {
                    rust_name: interface.name().to_owned(),
                    c_name: naming::type_name(interface.name()),
                    function_name: naming::function_name(interface.name()),
                    kind: HandleKind::ForeignCallbackInterface,
                    clone_symbol: None,
                    free_symbol: None,
                }),
        )
        .collect::<Vec<_>>();
    handles.sort_by(|left, right| left.rust_name.cmp(&right.rust_name));
    for handle in &handles {
        types.reserve_c_name(handle.c_name())?;
    }
    Ok(handles)
}

fn is_local(type_: &Type, crate_name: &str) -> bool {
    match type_ {
        Type::Object { module_path, .. } | Type::CallbackInterface { module_path, .. } => {
            module_path.split("::").next() == Some(crate_name)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use uniffi_bindgen::ComponentInterface;

    use super::{HandleKind, collect_object_handles};
    use crate::type_registry::TypeRegistry;

    #[test]
    fn collects_rust_objects_and_both_callback_interface_forms() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};

            interface Client { constructor(); };
            [Trait, WithForeign]
            interface Host { void execute(); };
            callback interface LegacyHost { void execute(); };
            ",
            "wallet_engine",
        )?;
        let mut types = TypeRegistry::collect(&component)?;
        let handles = collect_object_handles(&component, &mut types)?;

        assert_eq!(handles.len(), 3);
        assert_eq!(handles[0].rust_name(), "Client");
        assert_eq!(handles[0].c_name(), "WalletEngineClient");
        assert_eq!(handles[0].kind(), HandleKind::RustObject);
        assert_eq!(handles[0].function_name(), "client");
        assert_eq!(
            handles[0].clone_symbol(),
            Some("uniffi_wallet_engine_fn_clone_client")
        );
        assert_eq!(
            handles[0].free_symbol(),
            Some("uniffi_wallet_engine_fn_free_client")
        );
        assert_eq!(handles[1].rust_name(), "Host");
        assert_eq!(handles[1].kind(), HandleKind::ForeignCallbackInterface);
        assert_eq!(handles[1].clone_symbol(), None);
        assert_eq!(handles[1].free_symbol(), None);
        assert_eq!(handles[2].rust_name(), "LegacyHost");
        assert_eq!(handles[2].kind(), HandleKind::ForeignCallbackInterface);
        assert_eq!(handles[2].clone_symbol(), None);
        assert_eq!(handles[2].free_symbol(), None);
        Ok(())
    }
}
