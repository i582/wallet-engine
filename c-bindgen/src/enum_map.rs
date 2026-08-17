use std::collections::BTreeSet;

use anyhow::{Context, Result, ensure};
use uniffi_bindgen::ComponentInterface;

use crate::naming;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FlatEnum {
    rust_name: String,
    c_name: String,
    function_name: String,
    variants: Vec<FlatEnumVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FlatEnumVariant {
    rust_name: String,
    c_constant: String,
    public_value: u32,
    wire_tag: i32,
}

impl FlatEnum {
    pub(super) fn rust_name(&self) -> &str {
        &self.rust_name
    }

    pub(super) fn c_name(&self) -> &str {
        &self.c_name
    }

    pub(super) fn function_name(&self) -> &str {
        &self.function_name
    }

    pub(super) fn variants(&self) -> &[FlatEnumVariant] {
        &self.variants
    }
}

impl FlatEnumVariant {
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
}

pub(super) fn collect_flat_enums(component: &ComponentInterface) -> Result<Vec<FlatEnum>> {
    let mut enums = component
        .enum_definitions()
        .iter()
        .filter(|enum_| enum_.is_flat() && !component.is_name_used_as_error(enum_.name()))
        .map(|enum_| {
            let rust_name = enum_.name().to_owned();
            let c_name = naming::type_name(&rust_name);
            let function_name = naming::function_name(&rust_name);
            let mut constants = BTreeSet::new();
            let variants = enum_
                .variants()
                .iter()
                .enumerate()
                .map(|(index, variant)| {
                    let public_value = u32::try_from(index).with_context(|| {
                        format!("enum {rust_name} has too many variants for the public C ABI")
                    })?;
                    let wire_tag = i32::try_from(index.saturating_add(1)).with_context(|| {
                        format!("enum {rust_name} has too many variants for the UniFFI wire ABI")
                    })?;
                    let c_constant = naming::constant_name(&rust_name, variant.name());
                    ensure!(
                        constants.insert(c_constant.clone()),
                        "enum {rust_name} has C constant collision at {c_constant}"
                    );
                    Ok(FlatEnumVariant {
                        rust_name: variant.name().to_owned(),
                        c_constant,
                        public_value,
                        wire_tag,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            ensure!(!variants.is_empty(), "enum {rust_name} has no variants");
            Ok(FlatEnum {
                rust_name,
                c_name,
                function_name,
                variants,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    enums.sort_by(|left, right| left.rust_name.cmp(&right.rust_name));

    let mut c_names = BTreeSet::new();
    for enum_ in &enums {
        ensure!(
            c_names.insert(enum_.c_name.clone()),
            "flat enums produce duplicate C type {}",
            enum_.c_name
        );
    }
    Ok(enums)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use uniffi_bindgen::ComponentInterface;

    use super::collect_flat_enums;

    #[test]
    fn collects_only_non_error_enums_without_payloads() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r#"
            namespace wallet_engine {
                [Throws=Failure] void fallible();
            };

            enum Network { "mainnet", "testnet" };

            [Enum]
            interface SendAmount {
                Exact(u64 value);
                All();
            };

            [Error]
            enum Failure { "failed" };
            "#,
            "wallet_engine",
        )?;
        let enums = collect_flat_enums(&component)?;

        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].rust_name(), "Network");
        assert_eq!(enums[0].c_name(), "WalletEngineNetwork");
        assert_eq!(enums[0].variants()[0].public_value(), 0);
        assert_eq!(enums[0].variants()[0].wire_tag(), 1);
        assert_eq!(
            enums[0].variants()[1].c_constant(),
            "WALLET_ENGINE_NETWORK_TESTNET"
        );
        Ok(())
    }
}
