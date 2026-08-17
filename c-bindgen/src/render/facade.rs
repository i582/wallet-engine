use crate::model::BindingsModel;

pub(super) fn render(model: &BindingsModel) -> String {
    let mut facade = String::from(
        r#"#define WALLET_ENGINE_BUILD
#include "wallet_engine.h"

#include <limits.h>
#include <string.h>

uint32_t wallet_engine_abi_version(void) {
    return WALLET_ENGINE_ABI_VERSION;
}

uint32_t wallet_engine_uniffi_contract_version(void) {
    return WALLET_ENGINE_UNIFFI_CONTRACT_VERSION;
}
"#,
    );
    facade.push_str(&super::codec::render(model));
    facade
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn facade_includes_its_generated_header() -> anyhow::Result<()> {
        let model = crate::model::BindingsModel::from_components(&[
            uniffi_bindgen::ComponentInterface::new("wallet_engine"),
        ])?;
        let facade = render(&model);

        assert!(facade.contains("#include \"wallet_engine.h\""));
        assert!(facade.contains("uint32_t wallet_engine_abi_version(void)"));
        Ok(())
    }
}
