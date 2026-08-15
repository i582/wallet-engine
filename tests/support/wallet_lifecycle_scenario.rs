use std::collections::HashMap;
use std::sync::Arc;

use futures::executor::block_on;
use wallet_engine::{
    CreateWalletRequest, CreatedWallet, ImportWalletRequest, Network, RecoveryPhrase,
    SecretAccessReason, WalletDescriptor, WalletLifecycle, WalletLifecycleError,
};

use super::host::MemoryPlatformHost;

pub(crate) fn wallet_lifecycle_scenario(name: impl Into<String>) -> WalletLifecycleScenario {
    WalletLifecycleScenario {
        name: name.into(),
        steps: Vec::new(),
    }
}

pub(crate) fn create_wallet(
    operation: impl Into<String>,
    record_id: impl Into<String>,
    network: Network,
) -> LifecycleAction {
    LifecycleAction::Create {
        operation: operation.into(),
        request: CreateWalletRequest {
            record_id: record_id.into(),
            network,
        },
    }
}

pub(crate) fn import_wallet(
    operation: impl Into<String>,
    record_id: impl Into<String>,
    network: Network,
    recovery_words: Vec<String>,
) -> LifecycleAction {
    LifecycleAction::Import {
        operation: operation.into(),
        request: ImportWalletRequest {
            record_id: record_id.into(),
            network,
            recovery_words,
        },
    }
}

pub(crate) fn reveal_wallet(
    operation: impl Into<String>,
    descriptor_from: impl Into<String>,
) -> LifecycleAction {
    LifecycleAction::Reveal {
        operation: operation.into(),
        descriptor_from: descriptor_from.into(),
    }
}

pub(crate) fn delete_wallet(
    operation: impl Into<String>,
    descriptor_from: impl Into<String>,
) -> LifecycleAction {
    LifecycleAction::Delete {
        operation: operation.into(),
        descriptor_from: descriptor_from.into(),
    }
}

pub(crate) fn replace_protected_secret(
    target_descriptor: impl Into<String>,
    source_descriptor: impl Into<String>,
) -> LifecycleAction {
    LifecycleAction::ReplaceProtectedSecret {
        target_descriptor: target_descriptor.into(),
        source_descriptor: source_descriptor.into(),
    }
}

pub(crate) fn descriptor_is(
    operation: impl Into<String>,
    record_id: impl Into<String>,
    network: Network,
) -> LifecycleExpectation {
    LifecycleExpectation::DescriptorIs {
        operation: operation.into(),
        record_id: record_id.into(),
        network,
    }
}

pub(crate) fn descriptor_address_is(
    operation: impl Into<String>,
    address: impl Into<String>,
) -> LifecycleExpectation {
    LifecycleExpectation::DescriptorAddressIs {
        operation: operation.into(),
        address: address.into(),
    }
}

pub(crate) fn descriptor_addresses_differ(
    left: impl Into<String>,
    right: impl Into<String>,
) -> LifecycleExpectation {
    LifecycleExpectation::DescriptorAddressesDiffer {
        left: left.into(),
        right: right.into(),
    }
}

pub(crate) fn phrase_has_words(operation: impl Into<String>, count: usize) -> LifecycleExpectation {
    LifecycleExpectation::PhraseHasWords {
        operation: operation.into(),
        count,
    }
}

pub(crate) fn phrase_is(operation: impl Into<String>, words: Vec<String>) -> LifecycleExpectation {
    LifecycleExpectation::PhraseIs {
        operation: operation.into(),
        words,
    }
}

pub(crate) fn phrases_match(
    left: impl Into<String>,
    right: impl Into<String>,
) -> LifecycleExpectation {
    LifecycleExpectation::PhrasesMatch {
        left: left.into(),
        right: right.into(),
    }
}

pub(crate) fn protected_secret_is_stored(
    descriptor_from: impl Into<String>,
) -> LifecycleExpectation {
    LifecycleExpectation::ProtectedSecretIsStored {
        descriptor_from: descriptor_from.into(),
    }
}

pub(crate) fn protected_secret_was_revealed(
    descriptor_from: impl Into<String>,
) -> LifecycleExpectation {
    LifecycleExpectation::ProtectedSecretWasRevealed {
        descriptor_from: descriptor_from.into(),
    }
}

pub(crate) fn protected_secret_is_deleted(
    descriptor_from: impl Into<String>,
) -> LifecycleExpectation {
    LifecycleExpectation::ProtectedSecretIsDeleted {
        descriptor_from: descriptor_from.into(),
    }
}

pub(crate) const fn no_protected_secrets_were_stored() -> LifecycleExpectation {
    LifecycleExpectation::StoredSecretCount(0)
}

pub(crate) fn lifecycle_succeeds(operation: impl Into<String>) -> LifecycleExpectation {
    LifecycleExpectation::Success {
        operation: operation.into(),
    }
}

pub(crate) fn lifecycle_error(
    operation: impl Into<String>,
    expected: WalletLifecycleError,
) -> LifecycleExpectation {
    LifecycleExpectation::Error {
        operation: operation.into(),
        expected,
    }
}

pub(crate) enum LifecycleAction {
    Create {
        operation: String,
        request: CreateWalletRequest,
    },
    Import {
        operation: String,
        request: ImportWalletRequest,
    },
    Reveal {
        operation: String,
        descriptor_from: String,
    },
    Delete {
        operation: String,
        descriptor_from: String,
    },
    ReplaceProtectedSecret {
        target_descriptor: String,
        source_descriptor: String,
    },
}

pub(crate) enum LifecycleExpectation {
    DescriptorIs {
        operation: String,
        record_id: String,
        network: Network,
    },
    DescriptorAddressIs {
        operation: String,
        address: String,
    },
    DescriptorAddressesDiffer {
        left: String,
        right: String,
    },
    PhraseHasWords {
        operation: String,
        count: usize,
    },
    PhraseIs {
        operation: String,
        words: Vec<String>,
    },
    PhrasesMatch {
        left: String,
        right: String,
    },
    ProtectedSecretIsStored {
        descriptor_from: String,
    },
    ProtectedSecretWasRevealed {
        descriptor_from: String,
    },
    ProtectedSecretIsDeleted {
        descriptor_from: String,
    },
    StoredSecretCount(usize),
    Success {
        operation: String,
    },
    Error {
        operation: String,
        expected: WalletLifecycleError,
    },
}

enum LifecycleStep {
    When(LifecycleAction),
    Then(LifecycleExpectation),
}

pub(crate) struct WalletLifecycleScenario {
    name: String,
    steps: Vec<LifecycleStep>,
}

impl WalletLifecycleScenario {
    #[must_use]
    pub(crate) fn when(mut self, action: LifecycleAction) -> Self {
        self.steps.push(LifecycleStep::When(action));
        self
    }

    #[must_use]
    pub(crate) fn then(mut self, expectation: LifecycleExpectation) -> Self {
        self.steps.push(LifecycleStep::Then(expectation));
        self
    }

    pub(crate) fn run(self) {
        let host = Arc::new(MemoryPlatformHost::default());
        let lifecycle = WalletLifecycle::new(host.clone());
        let mut runner = WalletLifecycleRunner {
            lifecycle,
            host,
            results: HashMap::new(),
        };

        for (index, step) in self.steps.into_iter().enumerate() {
            let result = match step {
                LifecycleStep::When(action) => runner.execute(action),
                LifecycleStep::Then(expectation) => runner.assert(expectation),
            };
            if let Err(message) = result {
                panic!(
                    "scenario: {}\nstep {} failed:\n{}",
                    self.name,
                    index + 1,
                    message
                );
            }
        }
    }
}

enum LifecycleResult {
    Created(Result<CreatedWallet, WalletLifecycleError>),
    Descriptor(Result<WalletDescriptor, WalletLifecycleError>),
    Phrase(Result<RecoveryPhrase, WalletLifecycleError>),
    Unit(Result<(), WalletLifecycleError>),
}

struct WalletLifecycleRunner {
    lifecycle: Arc<WalletLifecycle>,
    host: Arc<MemoryPlatformHost>,
    results: HashMap<String, LifecycleResult>,
}

impl WalletLifecycleRunner {
    fn execute(&mut self, action: LifecycleAction) -> Result<(), String> {
        let (operation, result) = match action {
            LifecycleAction::Create { operation, request } => {
                let result = block_on(self.lifecycle.create_wallet(request));
                (operation, LifecycleResult::Created(result))
            }
            LifecycleAction::Import { operation, request } => {
                let result = block_on(self.lifecycle.import_wallet(request));
                (operation, LifecycleResult::Descriptor(result))
            }
            LifecycleAction::Reveal {
                operation,
                descriptor_from,
            } => {
                let descriptor = self.descriptor(&descriptor_from)?.clone();
                let result = block_on(self.lifecycle.reveal_recovery_phrase(descriptor));
                (operation, LifecycleResult::Phrase(result))
            }
            LifecycleAction::Delete {
                operation,
                descriptor_from,
            } => {
                let descriptor = self.descriptor(&descriptor_from)?.clone();
                let result = block_on(self.lifecycle.delete_wallet(descriptor));
                (operation, LifecycleResult::Unit(result))
            }
            LifecycleAction::ReplaceProtectedSecret {
                target_descriptor,
                source_descriptor,
            } => {
                let target = self.descriptor(&target_descriptor)?.secret_ref.clone();
                let source = self.descriptor(&source_descriptor)?.secret_ref.clone();
                self.host.replace_secret(&target, &source)?;
                return Ok(());
            }
        };

        if self.results.insert(operation.clone(), result).is_some() {
            return Err(format!("operation `{operation}` already exists"));
        }
        Ok(())
    }

    fn assert(&self, expectation: LifecycleExpectation) -> Result<(), String> {
        match expectation {
            LifecycleExpectation::DescriptorIs {
                operation,
                record_id,
                network,
            } => {
                let descriptor = self.descriptor(&operation)?;
                let expected_ref = format!("wallet:{record_id}:mnemonic");
                if descriptor.record_id == record_id
                    && descriptor.network == network
                    && descriptor.secret_ref.value == expected_ref
                    && !descriptor.address.is_empty()
                {
                    Ok(())
                } else {
                    Err(format!(
                        "descriptor `{operation}` did not preserve record, network, address, and secret reference"
                    ))
                }
            }
            LifecycleExpectation::DescriptorAddressIs { operation, address } => {
                let actual = &self.descriptor(&operation)?.address;
                if actual == &address {
                    Ok(())
                } else {
                    Err(format!("expected address `{address}`, got `{actual}`"))
                }
            }
            LifecycleExpectation::DescriptorAddressesDiffer { left, right } => {
                let left_address = &self.descriptor(&left)?.address;
                let right_address = &self.descriptor(&right)?.address;
                if left_address != right_address {
                    Ok(())
                } else {
                    Err(format!(
                        "expected `{left}` and `{right}` to derive different addresses"
                    ))
                }
            }
            LifecycleExpectation::PhraseHasWords { operation, count } => {
                let actual = self.phrase(&operation)?.split_ascii_whitespace().count();
                if actual == count {
                    Ok(())
                } else {
                    Err(format!("expected {count} words, got {actual}"))
                }
            }
            LifecycleExpectation::PhraseIs { operation, words } => {
                let actual = self.phrase(&operation)?;
                if actual == words.join(" ") {
                    Ok(())
                } else {
                    Err(format!(
                        "phrase `{operation}` did not preserve the imported words"
                    ))
                }
            }
            LifecycleExpectation::PhrasesMatch { left, right } => {
                if self.phrase(&left)? == self.phrase(&right)? {
                    Ok(())
                } else {
                    Err(format!("phrases `{left}` and `{right}` differ"))
                }
            }
            LifecycleExpectation::ProtectedSecretIsStored { descriptor_from } => {
                let secret_ref = &self.descriptor(&descriptor_from)?.secret_ref;
                if self.host.secret_exists(secret_ref)
                    && self.host.secret_requires_user_presence(secret_ref) == Some(true)
                {
                    Ok(())
                } else {
                    Err(format!(
                        "secret for `{descriptor_from}` was not stored with user presence"
                    ))
                }
            }
            LifecycleExpectation::ProtectedSecretWasRevealed { descriptor_from } => {
                let secret_ref = &self.descriptor(&descriptor_from)?.secret_ref;
                if self
                    .host
                    .secret_was_read_for(secret_ref, SecretAccessReason::RevealRecoveryPhrase)
                {
                    Ok(())
                } else {
                    Err(format!(
                        "secret for `{descriptor_from}` was not read for phrase reveal"
                    ))
                }
            }
            LifecycleExpectation::ProtectedSecretIsDeleted { descriptor_from } => {
                let secret_ref = &self.descriptor(&descriptor_from)?.secret_ref;
                if self.host.secret_exists(secret_ref) {
                    Err(format!("secret for `{descriptor_from}` still exists"))
                } else {
                    Ok(())
                }
            }
            LifecycleExpectation::StoredSecretCount(expected) => {
                let actual = self.host.stored_secret_count();
                if actual == expected {
                    Ok(())
                } else {
                    Err(format!("expected {expected} stored secrets, got {actual}"))
                }
            }
            LifecycleExpectation::Success { operation } => match self.results.get(&operation) {
                Some(LifecycleResult::Unit(Ok(()))) => Ok(()),
                Some(_) => Err(format!("operation `{operation}` did not succeed")),
                None => Err(format!("operation `{operation}` does not exist")),
            },
            LifecycleExpectation::Error {
                operation,
                expected,
            } => {
                let actual = self.error(&operation)?;
                if actual == &expected {
                    Ok(())
                } else {
                    Err(format!("expected {expected:?}, got {actual:?}"))
                }
            }
        }
    }

    fn descriptor(&self, operation: &str) -> Result<&WalletDescriptor, String> {
        match self.results.get(operation) {
            Some(LifecycleResult::Created(Ok(created))) => Ok(&created.descriptor),
            Some(LifecycleResult::Descriptor(Ok(descriptor))) => Ok(descriptor),
            Some(_) => Err(format!("operation `{operation}` has no descriptor")),
            None => Err(format!("operation `{operation}` does not exist")),
        }
    }

    fn phrase(&self, operation: &str) -> Result<&str, String> {
        match self.results.get(operation) {
            Some(LifecycleResult::Created(Ok(created))) => Ok(&created.recovery_phrase.phrase),
            Some(LifecycleResult::Phrase(Ok(phrase))) => Ok(&phrase.phrase),
            Some(_) => Err(format!("operation `{operation}` has no recovery phrase")),
            None => Err(format!("operation `{operation}` does not exist")),
        }
    }

    fn error(&self, operation: &str) -> Result<&WalletLifecycleError, String> {
        let error = match self.results.get(operation) {
            Some(LifecycleResult::Created(Err(error)))
            | Some(LifecycleResult::Descriptor(Err(error)))
            | Some(LifecycleResult::Phrase(Err(error)))
            | Some(LifecycleResult::Unit(Err(error))) => error,
            Some(_) => return Err(format!("operation `{operation}` succeeded")),
            None => return Err(format!("operation `{operation}` does not exist")),
        };
        Ok(error)
    }
}
