use super::*;

use rgb_lightning_node::NativeExternalSigner;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("Error converting JSON: {0}")]
    JSONConversion(#[from] serde_json::Error),

    #[error("Error converting hex: {0}")]
    HexConversion(String),

    #[error("Error parsing string: {0}")]
    StringParse(String),

    #[error("Error from rgb-lightning-node: {0:?}")]
    Rln(RlnError),

    #[error("Type mismatch")]
    TypeMismatch,

    /// A Rust panic was caught at the FFI boundary by `catch_panic`.
    /// Carries the label of the FFI entry point that was active when
    /// the panic occurred plus the formatted panic payload. Without
    /// this, panics from any depth (rgb-lib, LDK, BDK) would unwind
    /// across `extern "C"` and the runtime would abort via
    /// `panic_cannot_unwind`, taking the whole app process with it —
    /// a non-recoverable failure mode that was masking real bugs.
    #[error("FFI panic in {0}: {1}")]
    Panic(String, String),
}

impl From<RlnError> for Error {
    fn from(e: RlnError) -> Self {
        Error::Rln(e)
    }
}

impl COpaqueStruct {
    pub(crate) fn new<T: 'static>(other: T) -> Self {
        let mut hasher = DefaultHasher::new();
        TypeId::of::<T>().hash(&mut hasher);
        let ty = hasher.finish();

        COpaqueStruct {
            ptr: Box::into_raw(Box::new(other)) as *const c_void,
            ty,
        }
    }

    pub(crate) fn raw<T>(ptr: *const T) -> Self {
        COpaqueStruct {
            ptr: ptr as *const c_void,
            ty: 0,
        }
    }

    // Public helpers for non-C consumers (e.g. the napi-rs Node binding
    // in `rgb-lightning-node-nodejs`, which links against this crate as
    // a normal Rust dep). C / C++ callers don't need these — they work
    // off the `#[repr(C)]` layout directly.

    /// Null sentinel — used by Rust callers when moving a handle out of
    /// a struct (`std::mem::replace`) so the original slot is left in a
    /// safe-to-drop state.
    pub fn null() -> Self {
        COpaqueStruct {
            ptr: std::ptr::null(),
            ty: 0,
        }
    }

    /// On the `CResultValue::Err` branch of a `CResult` / `CResultString`
    /// the `inner.ptr` is actually a `*mut c_char` allocated by
    /// `string_to_ptr` and pointing at the formatted error message.
    /// Cast it back so consumers can read + free it with `rln_free_string`.
    pub fn as_err_string_ptr(&self) -> *mut c_char {
        self.ptr as *mut c_char
    }
}

pub(crate) trait CReturnType: Sized + 'static {
    #[allow(clippy::mut_from_ref)]
    fn from_opaque(other: &COpaqueStruct) -> Result<&mut Self, Error> {
        let mut hasher = DefaultHasher::new();
        TypeId::of::<Self>().hash(&mut hasher);
        let ty = hasher.finish();

        if other.ty != ty {
            return Err(Error::TypeMismatch);
        }

        let boxed = unsafe { Box::from_raw(other.ptr as *mut Self) };
        Ok(Box::leak(boxed))
    }
}
impl CReturnType for SdkNode {}
impl CReturnType for Arc<NativeExternalSigner> {}

// Format a c-ffi-crate `Error` for the FFI boundary.
//
// Only `Error::Rln(_)` cases carry a stashed APIError detail string —
// for those we drain the per-thread slot and append it so consumers
// see e.g. `Rln(Conflict): unsupported in external signer mode:
// issueassetnia` instead of just the coarse category. For non-RLN
// variants we drain (and discard) the slot too so stale residue from a
// prior call never leaks into a later unrelated error message — this
// keeps the slot strictly one-shot.
//
// Format: the `Rln(<Variant>)` prefix is generated from a typed match
// (not from `format!("{:?}", outer)` substring matching) so callers can
// still grep on the variant tag, but a rename of `RlnError` variants
// won't silently break the formatting. The trailing detail comes from
// `RlnError::Display` (`thiserror` `#[error("...")]` strings) plus the
// stashed APIError string from the daemon-side mapper.
fn format_error_for_ffi(e: &Error) -> String {
    let stashed = rgb_lightning_node::take_last_api_error_detail();
    match e {
        Error::Rln(inner) => {
            let tag = rln_variant_tag(inner);
            match stashed {
                Some(detail) => format!("Rln({tag}): {detail}"),
                None => format!("Rln({tag}): {inner}"),
            }
        }
        // For non-Rln variants the stashed slot, if any, is not ours —
        // it was drained above so it can't poison a future call. Use
        // `Display`, not `Debug`, so the message is the human-readable
        // form defined via `thiserror` annotations.
        other => format!("{other}"),
    }
}

/// Stable, grep-friendly tag for an `RlnError` variant. Kept as a
/// typed match so a UDL-level rename is a compile error here rather
/// than a silent string mismatch downstream.
fn rln_variant_tag(e: &RlnError) -> &'static str {
    match e {
        RlnError::NotInitialized => "NotInitialized",
        RlnError::InvalidRequest => "InvalidRequest",
        RlnError::NotFound => "NotFound",
        RlnError::Conflict => "Conflict",
        RlnError::Internal => "Internal",
    }
}

impl<T: 'static> From<Result<T, Error>> for CResult {
    fn from(other: Result<T, Error>) -> Self {
        match other {
            Ok(d) => {
                // Drain the detail slot on success too — otherwise a
                // failed call followed by a successful one would leave
                // stale residue that the next failure picks up.
                let _ = rgb_lightning_node::take_last_api_error_detail();
                CResult {
                    result: CResultValue::Ok,
                    inner: COpaqueStruct::new(d),
                }
            }
            Err(e) => CResult {
                result: CResultValue::Err,
                inner: COpaqueStruct::raw(string_to_ptr(format_error_for_ffi(&e))),
            },
        }
    }
}

impl From<Result<String, Error>> for CResultString {
    fn from(other: Result<String, Error>) -> Self {
        match other {
            Ok(d) => {
                let _ = rgb_lightning_node::take_last_api_error_detail();
                CResultString {
                    result: CResultValue::Ok,
                    inner: string_to_ptr(d),
                }
            }
            Err(e) => CResultString {
                result: CResultValue::Err,
                inner: string_to_ptr(format_error_for_ffi(&e)),
            },
        }
    }
}

impl From<Result<(), Error>> for CResultString {
    fn from(other: Result<(), Error>) -> Self {
        match other {
            Ok(()) => {
                let _ = rgb_lightning_node::take_last_api_error_detail();
                CResultString {
                    result: CResultValue::Ok,
                    inner: null_mut(),
                }
            }
            Err(e) => CResultString {
                result: CResultValue::Err,
                inner: string_to_ptr(format_error_for_ffi(&e)),
            },
        }
    }
}

pub(crate) fn ptr_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

pub(crate) fn string_to_ptr(other: String) -> *mut c_char {
    let cstr = match CString::new(other) {
        Ok(cstr) => cstr,
        Err(_) => CString::new(String::from(
            "Error converting string: contains a null-char",
        ))
        .unwrap(),
    };

    cstr.into_raw()
}

pub(crate) fn convert_optional_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        None
    } else {
        Some(ptr_to_string(ptr))
    }
}

pub(crate) fn require_handle(node: &COpaqueStruct) -> Result<&mut SdkNode, Error> {
    SdkNode::from_opaque(node)
}

/// Catch any panic that escapes from an FFI entry point and convert
/// it into `Error::Panic`. Without this wrapper, a `.unwrap()` panic
/// inside rgb-lib / LDK / BDK would unwind into the `extern "C"`
/// frame, hit `panic_cannot_unwind`, and abort the entire process.
///
/// ## Why this is an `#[inline(never)]` function pointer
///
/// Early versions of this helper used `impl FnOnce` and a generic
/// monomorphisation. The compiler then inlined the helper into every
/// `rln_*` entry point AND inlined the closure body — at which point
/// the LLVM optimiser eliminated the `__rust_try` landing pad
/// entirely (visible in the disassembly as zero `bl catch_unwind`
/// instructions in the wrapping function, with the only cleanup pad
/// being a direct call to `panic_cannot_unwind`). Net effect:
/// `catch_panic` looked like it was protecting the boundary, but a
/// real panic from rgb-lib still aborted the process.
///
/// Two safeguards prevent that regression:
///   1. `#[inline(never)]` keeps `catch_panic` as a real function
///      with its own frame and landing pad — the call site can't
///      see the implementation.
///   2. The closure is passed as `&mut dyn FnMut() -> _`. Erasing
///      the closure to a vtable forces the compiler to invoke it via
///      function pointer, so it can't be statically proven nounwind
///      and the `catch_unwind` machinery is preserved.
///
/// `AssertUnwindSafe` is required because most call sites capture
/// `&COpaqueStruct` / `*const c_char` — neither implements
/// `UnwindSafe`. This is safe in practice: a panic mid-call may
/// leave node-internal state inconsistent, but the SdkNode's own
/// invariants are protected by the tokio runtime + the daemon's
/// internal Mutex/`RwLock` poisoning logic (the daemon recovers
/// from poisoned locks rather than re-panicking).
#[inline(never)]
pub(crate) fn catch_panic<T>(
    label: &'static str,
    f: &mut dyn FnMut() -> Result<T, Error>,
) -> Result<T, Error> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f())) {
        Ok(r) => r,
        Err(payload) => {
            let msg = panic_payload_to_string(&payload);
            // Stash on stderr too — `tracing` may be filtered to a
            // level the host doesn't enable, but the panic should
            // always reach the log stream.
            eprintln!("[rln c-ffi] panic in {label}: {msg}");
            Err(Error::Panic(label.to_string(), msg))
        }
    }
}

fn panic_payload_to_string(payload: &Box<dyn std::any::Any + Send + 'static>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "panic with non-string payload".to_string()
}

pub(crate) fn require_signer(
    signer: &COpaqueStruct,
) -> Result<&mut Arc<NativeExternalSigner>, Error> {
    <Arc<NativeExternalSigner>>::from_opaque(signer)
}
