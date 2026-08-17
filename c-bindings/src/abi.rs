//! Versioning for the stable C ABI.

use std::ffi::c_char;

/// The major version of the Wallet Engine C ABI.
///
/// A breaking change to exported types or functions must increment this value.
pub const ABI_VERSION: u32 = 3;

/// The immediate result of a C ABI function call.
///
/// Domain failures use their own error types. This status is
/// reserved for failures at the language boundary itself.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletEngineAbiStatus {
    /// The synchronous ABI call completed successfully.
    Ok = 0,
    /// An argument is null, malformed, or outside its accepted range.
    InvalidArgument = 1,
    /// A string view does not contain valid UTF-8.
    InvalidUtf8 = 2,
    /// Rust caught a panic before it crossed the C boundary.
    Panic = 3,
}

/// A borrowed, non-NUL-terminated UTF-8 string.
///
/// `data` may be null only when `len` is zero. The pointed-to bytes must remain
/// readable for the complete duration of the ABI call receiving this view.
/// The receiver never frees or retains `data`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WalletEngineStringView {
    /// Pointer to the first UTF-8 byte.
    pub data: *const c_char,
    /// Number of bytes, excluding any optional trailing NUL.
    pub len: usize,
}

/// A borrowed sequence of bytes.
///
/// `data` may be null only when `len` is zero. The pointed-to bytes must remain
/// readable for the complete duration of the ABI call receiving this view.
/// The receiver never frees or retains `data`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WalletEngineBytesView {
    /// Pointer to the first byte.
    pub data: *const u8,
    /// Number of bytes in the sequence.
    pub len: usize,
}

impl WalletEngineStringView {
    /// Returns an empty view that does not reference any allocation.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            data: std::ptr::null(),
            len: 0,
        }
    }

    /// Validates this view and copies its UTF-8 contents into a Rust string.
    ///
    /// # Errors
    ///
    /// Returns [`WalletEngineAbiStatus::InvalidArgument`] when `data` is null
    /// for a non-empty view or the length cannot describe a Rust slice. Returns
    /// [`WalletEngineAbiStatus::InvalidUtf8`] when the bytes are not UTF-8.
    ///
    /// # Safety
    ///
    /// When `data` is non-null and `0 < len <= isize::MAX`, it must point to
    /// `len` consecutive bytes that remain readable for this call. Null or
    /// excessive-length views are rejected without dereferencing `data`.
    pub unsafe fn try_to_string(self) -> Result<String, WalletEngineAbiStatus> {
        // SAFETY: The caller guarantees that any accepted non-empty source
        // range is readable. `copy_bytes` validates the null and length cases
        // before it constructs a temporary slice.
        let bytes = unsafe { copy_bytes(self.data.cast(), self.len)? };
        String::from_utf8(bytes).map_err(|_| WalletEngineAbiStatus::InvalidUtf8)
    }
}

impl From<&str> for WalletEngineStringView {
    fn from(value: &str) -> Self {
        Self {
            data: value.as_ptr().cast(),
            len: value.len(),
        }
    }
}

impl WalletEngineBytesView {
    /// Returns an empty view that does not reference any allocation.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            data: std::ptr::null(),
            len: 0,
        }
    }

    /// Validates this view and copies its contents into a Rust vector.
    ///
    /// # Errors
    ///
    /// Returns [`WalletEngineAbiStatus::InvalidArgument`] when `data` is null
    /// for a non-empty view or the length cannot describe a Rust slice.
    ///
    /// # Safety
    ///
    /// When `data` is non-null and `0 < len <= isize::MAX`, it must point to
    /// `len` consecutive bytes that remain readable for this call. Null or
    /// excessive-length views are rejected without dereferencing `data`.
    pub unsafe fn try_to_vec(self) -> Result<Vec<u8>, WalletEngineAbiStatus> {
        // SAFETY: The caller guarantees that any accepted non-empty source
        // range is readable. `copy_bytes` validates the null and length cases
        // before it constructs a temporary slice.
        unsafe { copy_bytes(self.data, self.len) }
    }
}

impl From<&[u8]> for WalletEngineBytesView {
    fn from(value: &[u8]) -> Self {
        Self {
            data: value.as_ptr(),
            len: value.len(),
        }
    }
}

unsafe fn copy_bytes(data: *const u8, len: usize) -> Result<Vec<u8>, WalletEngineAbiStatus> {
    if len == 0 {
        return Ok(Vec::new());
    }

    if data.is_null() || len > isize::MAX as usize {
        return Err(WalletEngineAbiStatus::InvalidArgument);
    }

    // SAFETY: Null and excessive lengths were rejected above. The caller
    // guarantees that the remaining non-empty range is readable for `len`
    // bytes, as required by `slice::from_raw_parts`.
    Ok(unsafe { std::slice::from_raw_parts(data, len) }.to_vec())
}

/// Returns the major version implemented by the linked native library.
#[unsafe(no_mangle)]
pub const extern "C" fn wallet_engine_abi_version() -> u32 {
    ABI_VERSION
}
