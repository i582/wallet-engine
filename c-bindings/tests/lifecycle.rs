#![allow(unsafe_code)]

use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use wallet_engine_c::{
    WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE, WalletEngineAbiStatus, WalletEngineCompletionId,
    WalletEngineLifecycle, WalletEnginePlatformHostCallbacks, WalletEngineProtectedSecretStoreView,
    wallet_engine_lifecycle_free, wallet_engine_lifecycle_new,
};

#[derive(Default)]
struct TestContext {
    retains: AtomicUsize,
    releases: AtomicUsize,
}

unsafe fn test_context<'a>(context: *mut c_void) -> &'a TestContext {
    // SAFETY: Every callback table in this test uses a live `TestContext`.
    unsafe { &*context.cast::<TestContext>() }
}

unsafe extern "C" fn retain_context(context: *mut c_void) {
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    unsafe { test_context(context) }
        .retains
        .fetch_add(1, Ordering::Relaxed);
}

unsafe extern "C" fn release_context(context: *mut c_void) {
    // SAFETY: The callback table supplies a live `TestContext` pointer.
    unsafe { test_context(context) }
        .releases
        .fetch_add(1, Ordering::Relaxed);
}

const unsafe extern "C" fn store_protected_secret(
    _context: *mut c_void,
    _completion_id: WalletEngineCompletionId,
    _request: *const WalletEngineProtectedSecretStoreView,
) {
}

fn callback_table(context: &TestContext) -> WalletEnginePlatformHostCallbacks {
    WalletEnginePlatformHostCallbacks {
        struct_size: WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE,
        context: std::ptr::from_ref(context).cast_mut().cast(),
        retain: Some(retain_context),
        release: Some(release_context),
        store_protected_secret: Some(store_protected_secret),
    }
}

#[test]
fn lifecycle_new_validates_arguments_before_retaining_context() {
    let context = TestContext::default();
    let callbacks = callback_table(&context);

    // SAFETY: A null output pointer is explicitly accepted as an invalid
    // argument, and therefore is never dereferenced.
    let status = unsafe { wallet_engine_lifecycle_new(&callbacks, std::ptr::null_mut()) };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);

    let mut lifecycle = NonNull::<WalletEngineLifecycle>::dangling().as_ptr();
    // SAFETY: `lifecycle` is writable. A null host is rejected without being
    // dereferenced.
    let status = unsafe { wallet_engine_lifecycle_new(std::ptr::null(), &mut lifecycle) };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);
    assert!(lifecycle.is_null());

    let truncated = WalletEnginePlatformHostCallbacks {
        struct_size: WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE - 1,
        ..callbacks
    };
    lifecycle = NonNull::<WalletEngineLifecycle>::dangling().as_ptr();
    // SAFETY: The complete table is readable but deliberately advertises a
    // truncated prefix. `lifecycle` is writable.
    let status = unsafe { wallet_engine_lifecycle_new(&truncated, &mut lifecycle) };
    assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);
    assert!(lifecycle.is_null());

    for incomplete in [
        WalletEnginePlatformHostCallbacks {
            retain: None,
            ..callbacks
        },
        WalletEnginePlatformHostCallbacks {
            release: None,
            ..callbacks
        },
        WalletEnginePlatformHostCallbacks {
            store_protected_secret: None,
            ..callbacks
        },
    ] {
        lifecycle = NonNull::<WalletEngineLifecycle>::dangling().as_ptr();
        // SAFETY: The readable table deliberately omits one required callback.
        // `lifecycle` is writable.
        let status = unsafe { wallet_engine_lifecycle_new(&incomplete, &mut lifecycle) };
        assert_eq!(status, WalletEngineAbiStatus::InvalidArgument);
        assert!(lifecycle.is_null());
    }

    assert_eq!(context.retains.load(Ordering::Relaxed), 0);
    assert_eq!(context.releases.load(Ordering::Relaxed), 0);
}

#[test]
fn lifecycle_handle_owns_the_retained_host_context() {
    let context = TestContext::default();
    let callbacks = callback_table(&context);
    let mut lifecycle = std::ptr::null_mut();

    // SAFETY: The callback table, context, and output pointer remain valid for
    // the call and the resulting handle's lifetime.
    let status = unsafe { wallet_engine_lifecycle_new(&callbacks, &mut lifecycle) };
    assert_eq!(status, WalletEngineAbiStatus::Ok);
    assert!(!lifecycle.is_null());
    assert_eq!(context.retains.load(Ordering::Relaxed), 1);
    assert_eq!(context.releases.load(Ordering::Relaxed), 0);

    // SAFETY: This is the live handle returned above.
    unsafe { wallet_engine_lifecycle_free(lifecycle) };
    assert_eq!(context.releases.load(Ordering::Relaxed), 1);

    // SAFETY: Null is explicitly supported as a no-op.
    unsafe { wallet_engine_lifecycle_free(std::ptr::null_mut()) };
    assert_eq!(context.releases.load(Ordering::Relaxed), 1);
}
