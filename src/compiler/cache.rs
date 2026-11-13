use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use cranelift::codegen::incremental_cache::CacheKvStore;

// Cranelift's codegen cache, one file per fn under `.oi/`.
pub struct Store(PathBuf);

impl Store {
	pub fn open(root: &Path, sub: &str) -> Option<Self> {
		let off = std::env::var_os("OI_CACHE").is_some_and(|v| v == "0");
		(!off).then(|| Store(root.join(".oi").join(sub)))
	}

	// Deletes files no `get` or `insert` has touched in a while.
	pub fn sweep(&self) {
		let cutoff = SystemTime::now() - Duration::from_secs(14 * 24 * 60 * 60); // two weeks
		for entry in std::fs::read_dir(&self.0).into_iter().flatten().flatten() {
			if entry
				.metadata()
				.and_then(|meta| meta.modified())
				.is_ok_and(|time| time < cutoff)
			{
				let _ = std::fs::remove_file(entry.path());
			}
		}
	}

	fn entry(&self, key: &[u8]) -> PathBuf {
		self.0.join(key.iter().map(|b| format!("{b:02x}")).collect::<String>())
	}
}

impl CacheKvStore for Store {
	fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
		let path = self.entry(key);
		let _ = std::fs::File::open(&path).and_then(|file| file.set_modified(SystemTime::now()));
		std::fs::read(path).ok().map(Cow::Owned)
	}

	// Write, then rename.
	fn insert(&mut self, key: &[u8], val: Vec<u8>) {
		let tmp = self.0.join(format!("{}.tmp", std::process::id()));
		let _ = std::fs::create_dir_all(&self.0)
			.and_then(|_| std::fs::write(&tmp, val))
			.and_then(|_| std::fs::rename(&tmp, self.entry(key)));
	}
}
