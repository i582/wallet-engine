use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::{Context, Poll};

use async_trait::async_trait;
use js_sys::Promise;
use send_wrapper::SendWrapper;
use wallet_engine::{
    HttpCall, HttpCallId, HttpHostError, HttpHostErrorKind, HttpResponse, JournalCompareExchange,
    JournalCompareExchangeResult, JournalHostError, JournalHostErrorKind, JournalKey,
    JournalRecord, ProtectedSecretHostError, ProtectedSecretHostErrorKind, ProtectedSecretRead,
    ProtectedSecretRef, ProtectedSecretStore,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::error::{rejection_diagnostic, rejection_kind};
use crate::serde::{from_value, to_value};

thread_local! {
    static HTTP_HOSTS: RefCell<HashMap<u32, JsValue>> = RefCell::new(HashMap::new());
    static PLATFORM_HOSTS: RefCell<HashMap<u32, JsValue>> = RefCell::new(HashMap::new());
}

static NEXT_HOST_ID: AtomicU32 = AtomicU32::new(1);

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "WalletHttpHost")]
    pub type WalletHttpHost;

    #[wasm_bindgen(typescript_type = "WalletPlatformHost")]
    pub type WalletPlatformHost;
}

#[derive(Debug)]
pub(crate) struct HttpHostAdapter {
    id: u32,
}

impl HttpHostAdapter {
    pub(crate) fn register(host: WalletHttpHost) -> Self {
        let id = next_host_id();
        HTTP_HOSTS.with(|hosts| {
            hosts.borrow_mut().insert(id, host.into());
        });
        Self { id }
    }
}

impl Drop for HttpHostAdapter {
    fn drop(&mut self) {
        HTTP_HOSTS.with(|hosts| {
            hosts.borrow_mut().remove(&self.id);
        });
    }
}

#[async_trait]
impl wallet_engine::WalletHttpHost for HttpHostAdapter {
    async fn execute_http(&self, call: HttpCall) -> Result<HttpResponse, HttpHostError> {
        let argument = to_value(&call).map_err(|value| http_rejection(&value))?;
        let promise = invoke_promise(&HTTP_HOSTS, self.id, "executeHttp", &[argument])
            .map_err(|value| http_rejection(&value))?;
        let value = SendJsFuture::new(promise)
            .await
            .map_err(|value| http_rejection(&value))?;
        from_value(value).map_err(|value| http_rejection(&value))
    }

    async fn cancel_http(&self, call_id: HttpCallId) {
        let Ok(argument) = to_value(&call_id) else {
            return;
        };
        let Ok(promise) = invoke_promise(&HTTP_HOSTS, self.id, "cancelHttp", &[argument]) else {
            return;
        };
        let _ = SendJsFuture::new(promise).await;
    }
}

#[derive(Debug)]
pub(crate) struct PlatformHostAdapter {
    id: u32,
}

impl PlatformHostAdapter {
    pub(crate) fn register(host: WalletPlatformHost) -> Self {
        let id = next_host_id();
        PLATFORM_HOSTS.with(|hosts| {
            hosts.borrow_mut().insert(id, host.into());
        });
        Self { id }
    }
}

impl Drop for PlatformHostAdapter {
    fn drop(&mut self) {
        PLATFORM_HOSTS.with(|hosts| {
            hosts.borrow_mut().remove(&self.id);
        });
    }
}

#[async_trait]
impl wallet_engine::WalletPlatformHost for PlatformHostAdapter {
    async fn read_protected_secret(
        &self,
        request: ProtectedSecretRead,
    ) -> Result<Vec<u8>, ProtectedSecretHostError> {
        let argument = to_value(&request).map_err(|value| secret_rejection(&value))?;
        let value = platform_call(self.id, "readProtectedSecret", &[argument])
            .await
            .map_err(|value| secret_rejection(&value))?;
        from_value(value).map_err(|value| secret_rejection(&value))
    }

    async fn store_protected_secret(
        &self,
        request: ProtectedSecretStore,
    ) -> Result<(), ProtectedSecretHostError> {
        let argument = to_value(&request).map_err(|value| secret_rejection(&value))?;
        platform_call(self.id, "storeProtectedSecret", &[argument])
            .await
            .map_err(|value| secret_rejection(&value))?;
        Ok(())
    }

    async fn delete_protected_secret(
        &self,
        secret_ref: ProtectedSecretRef,
    ) -> Result<(), ProtectedSecretHostError> {
        let argument = to_value(&secret_ref).map_err(|value| secret_rejection(&value))?;
        platform_call(self.id, "deleteProtectedSecret", &[argument])
            .await
            .map_err(|value| secret_rejection(&value))?;
        Ok(())
    }

    async fn load_journal(
        &self,
        key: JournalKey,
    ) -> Result<Option<JournalRecord>, JournalHostError> {
        let argument = to_value(&key).map_err(|value| journal_rejection(&value))?;
        let value = platform_call(self.id, "loadJournal", &[argument])
            .await
            .map_err(|value| journal_rejection(&value))?;
        from_value(value).map_err(|value| journal_rejection(&value))
    }

    async fn compare_exchange_journal(
        &self,
        mutation: JournalCompareExchange,
    ) -> Result<JournalCompareExchangeResult, JournalHostError> {
        let argument = to_value(&mutation).map_err(|value| journal_rejection(&value))?;
        let value = platform_call(self.id, "compareExchangeJournal", &[argument])
            .await
            .map_err(|value| journal_rejection(&value))?;
        from_value(value).map_err(|value| journal_rejection(&value))
    }
}

async fn platform_call(id: u32, method: &str, arguments: &[JsValue]) -> Result<JsValue, JsValue> {
    let promise = invoke_promise(&PLATFORM_HOSTS, id, method, arguments)?;
    SendJsFuture::new(promise).await
}

fn invoke_promise(
    registry: &'static std::thread::LocalKey<RefCell<HashMap<u32, JsValue>>>,
    id: u32,
    method: &str,
    arguments: &[JsValue],
) -> Result<Promise, JsValue> {
    registry.with(|hosts| {
        let hosts = hosts.borrow();
        let host = hosts
            .get(&id)
            .ok_or_else(|| JsValue::from_str("JavaScript host is no longer registered"))?;
        let function = js_sys::Reflect::get(host, &JsValue::from_str(method))?
            .dyn_into::<js_sys::Function>()?;
        let result = match arguments {
            [] => function.call0(host)?,
            [first] => function.call1(host, first)?,
            _ => return Err(JsValue::from_str("unsupported host callback arity")),
        };
        result.dyn_into::<Promise>()
    })
}

fn next_host_id() -> u32 {
    NEXT_HOST_ID.fetch_add(1, Ordering::Relaxed)
}

struct SendJsFuture(SendWrapper<JsFuture>);

impl SendJsFuture {
    fn new(promise: Promise) -> Self {
        Self(SendWrapper::new(JsFuture::from(promise)))
    }
}

impl Future for SendJsFuture {
    type Output = Result<JsValue, JsValue>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut *Pin::into_inner(self).0).poll(context)
    }
}

fn http_rejection(value: &JsValue) -> HttpHostError {
    let kind = match rejection_kind(value).as_deref() {
        Some("offline") => HttpHostErrorKind::Offline,
        Some("timeout") => HttpHostErrorKind::Timeout,
        Some("connectionLost") => HttpHostErrorKind::ConnectionLost,
        Some("dns") => HttpHostErrorKind::Dns,
        Some("tls") => HttpHostErrorKind::Tls,
        Some("policyViolation") => HttpHostErrorKind::PolicyViolation,
        Some("responseTooLarge") => HttpHostErrorKind::ResponseTooLarge,
        Some("cancelled") => HttpHostErrorKind::Cancelled,
        _ => HttpHostErrorKind::Other,
    };
    HttpHostError::Failed {
        kind,
        diagnostic: rejection_diagnostic(value),
    }
}

fn secret_rejection(value: &JsValue) -> ProtectedSecretHostError {
    let kind = match rejection_kind(value).as_deref() {
        Some("notFound") => ProtectedSecretHostErrorKind::NotFound,
        Some("authenticationFailed") => ProtectedSecretHostErrorKind::AuthenticationFailed,
        Some("cancelled") => ProtectedSecretHostErrorKind::Cancelled,
        Some("unavailable") => ProtectedSecretHostErrorKind::Unavailable,
        Some("policyViolation") => ProtectedSecretHostErrorKind::PolicyViolation,
        _ => ProtectedSecretHostErrorKind::Other,
    };
    ProtectedSecretHostError::Failed {
        kind,
        diagnostic: rejection_diagnostic(value),
    }
}

fn journal_rejection(value: &JsValue) -> JournalHostError {
    let kind = match rejection_kind(value).as_deref() {
        Some("unavailable") => JournalHostErrorKind::Unavailable,
        Some("corruptData") => JournalHostErrorKind::CorruptData,
        Some("cancelled") => JournalHostErrorKind::Cancelled,
        _ => JournalHostErrorKind::Other,
    };
    JournalHostError::Failed {
        kind,
        diagnostic: rejection_diagnostic(value),
    }
}
