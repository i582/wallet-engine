pub(super) const fn render() -> &'static str {
    r#"#define WALLET_ENGINE_BUILD
#include "wallet_engine.h"

uint32_t wallet_engine_abi_version(void) {
    return WALLET_ENGINE_ABI_VERSION;
}

uint32_t wallet_engine_uniffi_contract_version(void) {
    return WALLET_ENGINE_UNIFFI_CONTRACT_VERSION;
}
"#
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn facade_includes_its_generated_header() {
        let facade = render();

        assert!(facade.contains("#include \"wallet_engine.h\""));
        assert!(facade.contains("uint32_t wallet_engine_abi_version(void)"));
    }
}
