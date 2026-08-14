//! Versioning for the stable C ABI.

/// The major version of the Wallet Engine C ABI.
///
/// A breaking change to exported types or functions must increment this value.
pub const ABI_VERSION: u32 = 1;

/// Returns the major version implemented by the linked native library.
#[unsafe(no_mangle)]
pub const extern "C" fn wallet_engine_abi_version() -> u32 {
    ABI_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_version_matches_header_constant() {
        assert_eq!(wallet_engine_abi_version(), ABI_VERSION);
    }
}
