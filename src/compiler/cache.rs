use std::borrow::Cow;
use std::path::{Path, PathBuf};

use cranelift::codegen::incremental_cache::CacheKvStore;

// Cranelift's codegen cache, one file per fn under `<root>/.oi/cache`.
pub struct Store(PathBuf);

impl Store {
	pub fn open(root: &Path) -> Option<Self> {
		let off = std::env::var_os("OI_CACHE").is_some_and(|v| v == "0");
		(!off).then(|| Store(root.join(".oi/cache")))
	}

	fn entry(&self, key: &[u8]) -> PathBuf {
		self.0.join(key.iter().map(|b| format!("{b:02x}")).collect::<String>())
	}
}

impl CacheKvStore for Store {
	fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
		std::fs::read(self.entry(key)).ok().map(Cow::Owned)
	}

	// Write, then rename.
	fn insert(&mut self, key: &[u8], val: Vec<u8>) {
		let tmp = self.0.join(format!("{}.tmp", std::process::id()));
		let _ = std::fs::create_dir_all(&self.0)
			.and_then(|_| std::fs::write(&tmp, val))
			.and_then(|_| std::fs::rename(&tmp, self.entry(key)));
	}
}
