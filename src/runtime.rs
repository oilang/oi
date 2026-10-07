pub use oi_runtime::*;

// Copy a runtime string out as an owned String.
pub(crate) unsafe fn str_string(s: *const StrHeader) -> String {
	String::from_utf8_lossy(unsafe { str_bytes(s) }).into_owned()
}
