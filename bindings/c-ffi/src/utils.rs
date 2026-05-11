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

impl<T: 'static, E> From<Result<T, E>> for CResult
where
    E: std::fmt::Debug,
{
    fn from(other: Result<T, E>) -> Self {
        match other {
            Ok(d) => CResult {
                result: CResultValue::Ok,
                inner: COpaqueStruct::new(d),
            },
            Err(e) => CResult {
                result: CResultValue::Err,
                inner: COpaqueStruct::raw(string_to_ptr(format!("{e:?}"))),
            },
        }
    }
}

impl From<Result<String, Error>> for CResultString {
    fn from(other: Result<String, Error>) -> Self {
        match other {
            Ok(d) => CResultString {
                result: CResultValue::Ok,
                inner: string_to_ptr(d),
            },
            Err(e) => CResultString {
                result: CResultValue::Err,
                inner: string_to_ptr(format!("{e:?}")),
            },
        }
    }
}

impl From<Result<(), Error>> for CResultString {
    fn from(other: Result<(), Error>) -> Self {
        match other {
            Ok(()) => CResultString {
                result: CResultValue::Ok,
                inner: null_mut(),
            },
            Err(e) => CResultString {
                result: CResultValue::Err,
                inner: string_to_ptr(format!("{e:?}")),
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

pub(crate) fn require_signer(
    signer: &COpaqueStruct,
) -> Result<&mut Arc<NativeExternalSigner>, Error> {
    <Arc<NativeExternalSigner>>::from_opaque(signer)
}
