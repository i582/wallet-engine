use uniffi_bindgen::{ComponentInterface, interface::Literal};

use crate::bindings::cpp::{
    CodeType, gen_cpp::filters::CppCodeOracle, gen_cpp::filters::external_namespace_prefix,
};

#[derive(Debug)]
pub(crate) struct RecordCodeType {
    id: String,
    module_path: String,
}

impl RecordCodeType {
    pub(crate) fn new(id: String, module_path: String) -> Self {
        Self { id, module_path }
    }
}

impl CodeType for RecordCodeType {
    fn type_label(&self, ci: &ComponentInterface) -> String {
        format!(
            "{}{}",
            external_namespace_prefix(ci, &self.module_path),
            CppCodeOracle.class_name(&self.id)
        )
    }

    fn canonical_name(&self) -> String {
        format!("Type{}", self.id)
    }

    fn literal(&self, _literal: &Literal, _ci: &ComponentInterface) -> String {
        unreachable!();
    }
}
