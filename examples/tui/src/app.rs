//! Application state and keyboard-driven wallet operations.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use num_bigint::BigUint;
use wallet_engine::{
    CreateWalletRequest, CreatedWallet, ImportWalletRequest, Network, NonEmptyString,
    ProviderConfig, SendAmount, SendPhase, SendPreviewRequest, SendRequest, TonAddressString,
    WalletClient, WalletClientConfig, WalletDescriptor, WalletLifecycle, WalletSnapshot,
};

use crate::http_host::ReqwestHttpHost;
use crate::storage::DiskStore;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputField {
    Destination,
    Amount,
}

pub(crate) enum Screen {
    Welcome,
    Recovery(CreatedWallet),
    Import,
    Dashboard,
    Send,
    ConfirmDelete,
}

pub(crate) struct App {
    pub(crate) screen: Screen,
    pub(crate) snapshot: Option<WalletSnapshot>,
    pub(crate) status: Option<String>,
    pub(crate) import_words: String,
    pub(crate) send_destination: String,
    pub(crate) send_amount: String,
    pub(crate) input_field: InputField,
    store: Arc<DiskStore>,
    http_host: Arc<ReqwestHttpHost>,
    lifecycle: Arc<WalletLifecycle>,
    client: Option<Arc<WalletClient>>,
    descriptor: Option<WalletDescriptor>,
    quit: bool,
}

impl App {
    pub(crate) async fn new(
        store: Arc<DiskStore>,
        http_host: Arc<ReqwestHttpHost>,
        lifecycle: Arc<WalletLifecycle>,
    ) -> Self {
        let mut app = Self {
            screen: Screen::Welcome,
            snapshot: None,
            status: None,
            import_words: String::new(),
            send_destination: String::new(),
            send_amount: String::new(),
            input_field: InputField::Destination,
            store,
            http_host,
            lifecycle,
            client: None,
            descriptor: None,
            quit: false,
        };

        match app.store.wallet() {
            Ok(Some(descriptor)) => app.open_wallet(descriptor).await,
            Ok(None) => {}
            Err(error) => app.status = Some(error.to_string()),
        }

        app
    }

    pub(crate) const fn should_quit(&self) -> bool {
        self.quit
    }

    pub(crate) fn descriptor(&self) -> Option<&WalletDescriptor> {
        self.descriptor.as_ref()
    }

    pub(crate) async fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }

        match self.screen {
            Screen::Welcome => self.handle_welcome(key).await,
            Screen::Recovery(_) => self.handle_recovery(key).await,
            Screen::Import => self.handle_import(key).await,
            Screen::Dashboard => self.handle_dashboard(key).await,
            Screen::Send => self.handle_send(key).await,
            Screen::ConfirmDelete => self.handle_delete_confirmation(key).await,
        }
    }

    pub(crate) async fn shutdown(&mut self) {
        if let Some(client) = self.client.take() {
            let _ = client.shutdown().await;
        }
    }

    async fn handle_welcome(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('c') => self.create_wallet().await,
            KeyCode::Char('i') => {
                self.status = None;
                self.import_words.clear();
                self.screen = Screen::Import;
            }
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            _ => {}
        }
    }

    async fn handle_recovery(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.confirm_created_wallet().await,
            KeyCode::Esc => self.discard_created_wallet().await,
            _ => {}
        }
    }

    async fn handle_import(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.import_wallet().await,
            KeyCode::Esc => {
                self.import_words.clear();
                self.status = None;
                self.screen = Screen::Welcome;
            }
            KeyCode::Backspace => {
                self.import_words.pop();
            }
            KeyCode::Char(character) => self.import_words.push(character),
            _ => {}
        }
    }

    async fn handle_dashboard(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('c') => self.copy_address(),
            KeyCode::Char('r') => self.refresh().await,
            KeyCode::Char('l') => self.load_more().await,
            KeyCode::Char('s') => {
                self.send_destination.clear();
                self.send_amount.clear();
                self.input_field = InputField::Destination;
                self.status = None;
                self.screen = Screen::Send;
            }
            KeyCode::Char('d') => self.screen = Screen::ConfirmDelete,
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            _ => {}
        }
    }

    fn copy_address(&mut self) {
        let Some(descriptor) = &self.descriptor else {
            self.status = Some("Wallet address is unavailable".to_owned());
            return;
        };

        self.status = Some(
            match arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.set_text(descriptor.address.to_string()))
            {
                Ok(()) => "Address copied".to_owned(),
                Err(error) => format!("Could not copy address: {error}"),
            },
        );
    }

    async fn handle_send(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.status = None;
                self.screen = Screen::Dashboard;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.input_field = match self.input_field {
                    InputField::Destination => InputField::Amount,
                    InputField::Amount => InputField::Destination,
                };
            }
            KeyCode::Enter if self.input_field == InputField::Destination => {
                self.input_field = InputField::Amount;
            }
            KeyCode::Enter => self.send().await,
            KeyCode::Backspace => match self.input_field {
                InputField::Destination => {
                    self.send_destination.pop();
                }
                InputField::Amount => {
                    self.send_amount.pop();
                }
            },
            KeyCode::Char(character) => match self.input_field {
                InputField::Destination if !character.is_whitespace() => {
                    self.send_destination.push(character);
                }
                InputField::Amount if character.is_ascii_digit() || character == '.' => {
                    self.send_amount.push(character);
                }
                _ => {}
            },
            _ => {}
        }
    }

    async fn handle_delete_confirmation(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') => self.delete_wallet().await,
            KeyCode::Char('n') | KeyCode::Esc => self.screen = Screen::Dashboard,
            _ => {}
        }
    }

    async fn create_wallet(&mut self) {
        self.status = Some("Creating wallet…".to_owned());
        let request = CreateWalletRequest {
            record_id: new_id("wallet"),
            network: Network::Testnet,
        };
        match self.lifecycle.create_wallet(request).await {
            Ok(created) => {
                self.status = None;
                self.screen = Screen::Recovery(created);
            }
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    async fn confirm_created_wallet(&mut self) {
        let Screen::Recovery(created) = &self.screen else {
            return;
        };
        let descriptor = created.descriptor.clone();
        match self.store.save_wallet(descriptor.clone()) {
            Ok(()) => self.open_wallet(descriptor).await,
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    async fn discard_created_wallet(&mut self) {
        let Screen::Recovery(created) = &self.screen else {
            return;
        };
        let descriptor = created.descriptor.clone();
        if let Err(error) = self.lifecycle.delete_wallet(descriptor).await {
            self.status = Some(error.to_string());
            return;
        }
        self.status = None;
        self.screen = Screen::Welcome;
    }

    async fn import_wallet(&mut self) {
        let words = self
            .import_words
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        if words.len() != 24 {
            self.status = Some("Enter exactly 24 recovery words".to_owned());
            return;
        }

        self.status = Some("Importing wallet…".to_owned());
        let request = ImportWalletRequest {
            record_id: new_id("wallet"),
            network: Network::Testnet,
            recovery_words: words,
        };
        match self.lifecycle.import_wallet(request).await {
            Ok(descriptor) => match self.store.save_wallet(descriptor.clone()) {
                Ok(()) => self.open_wallet(descriptor).await,
                Err(error) => self.status = Some(error.to_string()),
            },
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    async fn open_wallet(&mut self, descriptor: WalletDescriptor) {
        if let Some(client) = self.client.take() {
            let _ = client.shutdown().await;
        }

        let record_id = match NonEmptyString::try_from(descriptor.record_id.clone()) {
            Ok(record_id) => record_id,
            Err(error) => {
                self.status = Some(error.to_string());
                return;
            }
        };
        let config = WalletClientConfig {
            record_id,
            address: descriptor.address.clone(),
            public_key: descriptor.public_key.clone(),
            local_secret_ref: Some(descriptor.secret_ref.clone()),
            network: descriptor.network,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            providers: ProviderConfig::standard(descriptor.network),
        };
        match WalletClient::new(config, self.http_host.clone(), self.store.clone()) {
            Ok(client) => {
                self.snapshot = client.snapshot().ok();
                self.client = Some(client);
                self.descriptor = Some(descriptor);
                self.screen = Screen::Dashboard;
                self.refresh().await;
            }
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    async fn refresh(&mut self) {
        let Some(client) = self.client.clone() else {
            self.status = Some("Wallet client is unavailable".to_owned());
            return;
        };
        self.status = Some("Refreshing…".to_owned());
        match client.refresh().await {
            Ok(update) => {
                self.snapshot = Some(update.snapshot);
                self.status = None;
            }
            Err(error) => {
                self.snapshot = client.snapshot().ok();
                self.status = Some(error.to_string());
            }
        }
    }

    async fn load_more(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        self.status = Some("Loading history…".to_owned());
        match client.load_more_activity().await {
            Ok(update) => {
                self.snapshot = Some(update.snapshot);
                self.status = None;
            }
            Err(error) => self.status = Some(error.to_string()),
        }
    }

    async fn send(&mut self) {
        let Some(client) = self.client.clone() else {
            self.status = Some("Wallet client is unavailable".to_owned());
            return;
        };
        let amount_nanograms = match parse_gram_amount(&self.send_amount) {
            Ok(amount) => amount,
            Err(message) => {
                self.status = Some(message);
                return;
            }
        };

        let amount = match SendAmount::exact(amount_nanograms) {
            Ok(amount) => amount,
            Err(error) => {
                self.status = Some(error.to_string());
                return;
            }
        };

        let destination = match TonAddressString::try_from(self.send_destination.clone()) {
            Ok(destination) => destination,
            Err(error) => {
                self.status = Some(error.to_string());
                return;
            }
        };

        self.status = Some("Checking transfer…".to_owned());
        match client
            .preview_send(SendPreviewRequest {
                destination: destination.clone(),
                amount: amount.clone(),
                comment: None,
            })
            .await
        {
            Ok(preview) => preview,
            Err(error) => {
                self.status = Some(error.to_string());
                return;
            }
        };

        self.status = Some("Signing and submitting…".to_owned());
        let operation_id = match NonEmptyString::try_from(new_id("send")) {
            Ok(operation_id) => operation_id,
            Err(error) => {
                self.status = Some(error.to_string());
                return;
            }
        };
        let request = SendRequest {
            operation_id,
            destination,
            amount,
            comment: None,
        };
        match client.send(request).await {
            Ok(result) => {
                self.snapshot = client.snapshot().ok();
                self.status = Some(match result.phase {
                    SendPhase::Submitted => "Transfer submitted".to_owned(),
                    SendPhase::SubmissionUnknown => {
                        "Submission is unknown. Do not send it again.".to_owned()
                    }
                    phase => format!("Transfer finished: {phase:?}"),
                });
                self.screen = Screen::Dashboard;
                if result.phase == SendPhase::Submitted {
                    self.refresh().await;
                }
            }
            Err(error) => {
                self.snapshot = client.snapshot().ok();
                self.status = Some(error.to_string());
            }
        }
    }

    async fn delete_wallet(&mut self) {
        let Some(descriptor) = self.descriptor.clone() else {
            return;
        };
        if let Some(client) = self.client.take() {
            let _ = client.shutdown().await;
        }
        if let Err(error) = self.lifecycle.delete_wallet(descriptor).await {
            self.status = Some(error.to_string());
            self.screen = Screen::Dashboard;
            return;
        }
        if let Err(error) = self.store.clear_wallet() {
            self.status = Some(error.to_string());
            self.screen = Screen::Dashboard;
            return;
        }

        self.descriptor = None;
        self.snapshot = None;
        self.status = None;
        self.screen = Screen::Welcome;
    }
}

fn new_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{prefix}-{nanos}")
}

fn parse_gram_amount(value: &str) -> Result<String, String> {
    let value = value.trim();
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or_default();
    if value.is_empty()
        || parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 9
    {
        return Err("Enter a nonnegative amount with at most 9 decimal places".to_owned());
    }

    let whole = whole
        .parse::<BigUint>()
        .map_err(|_| "Amount is too large".to_owned())?;
    let fraction = format!("{fraction:0<9}")
        .parse::<BigUint>()
        .map_err(|_| "Amount is invalid".to_owned())?;
    let nanograms = whole * BigUint::from(1_000_000_000_u64) + fraction;
    Ok(nanograms.to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_gram_amount;

    #[test]
    fn parses_gram_amount_without_losing_precision() {
        assert_eq!(parse_gram_amount("1").as_deref(), Ok("1000000000"));
        assert_eq!(parse_gram_amount("0").as_deref(), Ok("0"));
        assert_eq!(parse_gram_amount("0.000000001").as_deref(), Ok("1"));
        assert_eq!(parse_gram_amount("12.3405").as_deref(), Ok("12340500000"));
    }

    #[test]
    fn rejects_invalid_gram_amount() {
        assert!(parse_gram_amount("1.0000000001").is_err());
        assert!(parse_gram_amount("1.2.3").is_err());
        assert!(parse_gram_amount("-1").is_err());
    }
}
