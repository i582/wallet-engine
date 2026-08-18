//! Small testnet-only storage adapter for the terminal example.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use wallet_engine::{
    JournalCompareExchange, JournalCompareExchangeResult, JournalHostError, JournalHostErrorKind,
    JournalKey, JournalRecord, ProtectedSecretHostError, ProtectedSecretHostErrorKind,
    ProtectedSecretRead, ProtectedSecretRef, ProtectedSecretStore, WalletDescriptor,
    WalletPlatformHost,
};

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredState {
    wallet: Option<WalletDescriptor>,
    secrets: HashMap<String, Vec<u8>>,
    journals: HashMap<String, JournalRecord>,
}

pub(crate) struct DiskStore {
    path: PathBuf,
    state: Mutex<StoredState>,
}

impl std::fmt::Debug for DiskStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DiskStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl DiskStore {
    pub(crate) fn open_default() -> Result<Self> {
        let root = std::env::var_os("WALLET_ENGINE_TUI_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".wallet-engine-tui"))
            })
            .unwrap_or_else(|| PathBuf::from(".wallet-engine-tui"));
        fs::create_dir_all(&root).context("failed to create the wallet data directory")?;
        set_private_directory_permissions(&root)?;

        let path = root.join("wallet.json");
        let state = if path.exists() {
            let bytes = fs::read(&path).context("failed to read the wallet data file")?;
            serde_json::from_slice(&bytes).context("the wallet data file is invalid")?
        } else {
            StoredState::default()
        };

        Ok(Self {
            path,
            state: Mutex::new(state),
        })
    }

    pub(crate) fn wallet(&self) -> Result<Option<WalletDescriptor>> {
        Ok(self.lock()?.wallet.clone())
    }

    pub(crate) fn save_wallet(&self, wallet: WalletDescriptor) -> Result<()> {
        let mut state = self.lock()?;
        state.wallet = Some(wallet);
        self.persist(&state)
    }

    pub(crate) fn clear_wallet(&self) -> Result<()> {
        let mut state = self.lock()?;
        state.wallet = None;
        self.persist(&state)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, StoredState>> {
        self.state
            .lock()
            .map_err(|_| anyhow::anyhow!("wallet storage lock is poisoned"))
    }

    fn persist(&self, state: &StoredState) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(state).context("failed to encode wallet data")?;
        let temporary = self.path.with_extension("json.tmp");
        let mut file = private_file(&temporary)?;
        file.write_all(&bytes)
            .context("failed to write wallet data")?;
        file.sync_all().context("failed to flush wallet data")?;
        drop(file);
        fs::rename(&temporary, &self.path).context("failed to replace wallet data")?;

        if let Some(parent) = self.path.parent() {
            File::open(parent)
                .and_then(|directory| directory.sync_all())
                .context("failed to flush the wallet data directory")?;
        }

        Ok(())
    }
}

#[async_trait]
impl WalletPlatformHost for DiskStore {
    async fn read_protected_secret(
        &self,
        request: ProtectedSecretRead,
    ) -> Result<Vec<u8>, ProtectedSecretHostError> {
        let state = self.lock().map_err(secret_storage_error)?;
        state
            .secrets
            .get(&request.secret_ref.value)
            .cloned()
            .ok_or_else(|| ProtectedSecretHostError::Failed {
                kind: ProtectedSecretHostErrorKind::NotFound,
                diagnostic: "recovery phrase is not stored".to_owned(),
            })
    }

    async fn store_protected_secret(
        &self,
        request: ProtectedSecretStore,
    ) -> Result<(), ProtectedSecretHostError> {
        let mut state = self.lock().map_err(secret_storage_error)?;
        let _ = state
            .secrets
            .insert(request.secret_ref.value, request.bytes);
        self.persist(&state).map_err(secret_storage_error)
    }

    async fn delete_protected_secret(
        &self,
        secret_ref: ProtectedSecretRef,
    ) -> Result<(), ProtectedSecretHostError> {
        let mut state = self.lock().map_err(secret_storage_error)?;
        let _ = state.secrets.remove(&secret_ref.value);
        self.persist(&state).map_err(secret_storage_error)
    }

    async fn load_journal(
        &self,
        key: JournalKey,
    ) -> Result<Option<JournalRecord>, JournalHostError> {
        let state = self.lock().map_err(journal_storage_error)?;
        Ok(state.journals.get(&journal_key(&key)).cloned())
    }

    async fn compare_exchange_journal(
        &self,
        mutation: JournalCompareExchange,
    ) -> Result<JournalCompareExchangeResult, JournalHostError> {
        let mut state = self.lock().map_err(journal_storage_error)?;
        let key = journal_key(&mutation.key);
        let current = state.journals.get(&key).cloned();
        let current_version = current.as_ref().map(|record| record.version);

        if current_version != mutation.expected_version {
            return Ok(JournalCompareExchangeResult {
                applied: false,
                current,
            });
        }

        let _ = state.journals.insert(key, mutation.replacement.clone());
        self.persist(&state).map_err(journal_storage_error)?;
        Ok(JournalCompareExchangeResult {
            applied: true,
            current: Some(mutation.replacement),
        })
    }
}

fn journal_key(key: &JournalKey) -> String {
    format!("{}:{}", key.record_id, key.slot)
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "used directly as an anyhow Result::map_err adapter"
)]
fn secret_storage_error(error: anyhow::Error) -> ProtectedSecretHostError {
    ProtectedSecretHostError::Failed {
        kind: ProtectedSecretHostErrorKind::Unavailable,
        diagnostic: error.to_string(),
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "used directly as an anyhow Result::map_err adapter"
)]
fn journal_storage_error(error: anyhow::Error) -> JournalHostError {
    JournalHostError::Failed {
        kind: JournalHostErrorKind::Unavailable,
        diagnostic: error.to_string(),
    }
}

fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    let _ = options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        let _ = options.mode(0o600);
    }
    options
        .open(path)
        .context("failed to open the temporary wallet data file")
}

fn set_private_directory_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .context("failed to protect the wallet data directory")?;
    }
    Ok(())
}
