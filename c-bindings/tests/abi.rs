#![allow(unsafe_code)]

use std::{ffi::c_char, mem::size_of};

use wallet_engine_c::{
    ABI_VERSION, WalletEngineAbiStatus, WalletEngineBytesView, WalletEngineStringView,
    wallet_engine_abi_version,
};

#[test]
fn exported_version_matches_header_constant() {
    assert_eq!(wallet_engine_abi_version(), ABI_VERSION);
}

#[test]
fn status_values_and_layout_are_stable() {
    assert_eq!(WalletEngineAbiStatus::Ok as u32, 0);
    assert_eq!(WalletEngineAbiStatus::InvalidArgument as u32, 1);
    assert_eq!(WalletEngineAbiStatus::InvalidUtf8 as u32, 2);
    assert_eq!(WalletEngineAbiStatus::Panic as u32, 3);
    assert_eq!(size_of::<WalletEngineAbiStatus>(), 4);
}

#[test]
fn borrowed_views_have_pointer_length_layout() {
    assert_eq!(
        size_of::<WalletEngineStringView>(),
        size_of::<(*const c_char, usize)>()
    );
    assert_eq!(
        size_of::<WalletEngineBytesView>(),
        size_of::<(*const u8, usize)>()
    );
}

#[test]
fn borrowed_views_copy_valid_values() {
    let text = b"wallet-engine";
    let string_view = WalletEngineStringView {
        data: text.as_ptr().cast(),
        len: text.len(),
    };
    let bytes_view = WalletEngineBytesView {
        data: text.as_ptr(),
        len: text.len(),
    };

    // SAFETY: Both views point to the complete, live `text` array.
    let string = unsafe { string_view.try_to_string() };
    assert_eq!(string.as_deref(), Ok("wallet-engine"));
    // SAFETY: Both views point to the complete, live `text` array.
    let bytes = unsafe { bytes_view.try_to_vec() };
    assert_eq!(bytes.as_deref(), Ok(text.as_slice()));
}

#[test]
fn null_empty_views_are_valid() {
    let string_view = WalletEngineStringView {
        data: std::ptr::null(),
        len: 0,
    };
    let bytes_view = WalletEngineBytesView {
        data: std::ptr::null(),
        len: 0,
    };

    // SAFETY: Empty views do not dereference their data pointers.
    let string = unsafe { string_view.try_to_string() };
    assert_eq!(string.as_deref(), Ok(""));
    // SAFETY: Empty views do not dereference their data pointers.
    let bytes = unsafe { bytes_view.try_to_vec() };
    assert_eq!(bytes, Ok(Vec::new()));
}

#[test]
fn null_non_empty_views_are_rejected() {
    let string_view = WalletEngineStringView {
        data: std::ptr::null(),
        len: 1,
    };
    let bytes_view = WalletEngineBytesView {
        data: std::ptr::null(),
        len: 1,
    };

    // SAFETY: The helpers reject null non-empty views before dereferencing.
    let string = unsafe { string_view.try_to_string() };
    assert_eq!(string, Err(WalletEngineAbiStatus::InvalidArgument));
    // SAFETY: The helpers reject null non-empty views before dereferencing.
    let bytes = unsafe { bytes_view.try_to_vec() };
    assert_eq!(bytes, Err(WalletEngineAbiStatus::InvalidArgument));
}

#[test]
fn invalid_utf8_is_rejected() {
    let bytes = [0xff];
    let view = WalletEngineStringView {
        data: bytes.as_ptr().cast(),
        len: bytes.len(),
    };

    // SAFETY: The view points to the complete, live `bytes` array.
    let string = unsafe { view.try_to_string() };
    assert_eq!(string, Err(WalletEngineAbiStatus::InvalidUtf8));
}

#[test]
fn excessive_view_lengths_are_rejected() {
    let string_view = WalletEngineStringView {
        data: std::ptr::NonNull::<c_char>::dangling().as_ptr(),
        len: isize::MAX as usize + 1,
    };
    let bytes_view = WalletEngineBytesView {
        data: std::ptr::NonNull::<u8>::dangling().as_ptr(),
        len: isize::MAX as usize + 1,
    };

    // SAFETY: Excessive lengths are rejected before the dangling pointers are
    // dereferenced.
    let string = unsafe { string_view.try_to_string() };
    assert_eq!(string, Err(WalletEngineAbiStatus::InvalidArgument));
    // SAFETY: Excessive lengths are rejected before the dangling pointers are
    // dereferenced.
    let bytes = unsafe { bytes_view.try_to_vec() };
    assert_eq!(bytes, Err(WalletEngineAbiStatus::InvalidArgument));
}
