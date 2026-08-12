use parking_lot::Mutex;
use std::{collections::HashMap, fs, path::Path, time::UNIX_EPOCH};

/// Keeps the resident set small: base64 PNG icons are a few kilobytes each, and
/// no view shows more than a few dozen applications at a time.
const CAPACITY: usize = 192;

/// Identifies the exact file an icon was extracted from, so a reinstall that
/// changes the executable invalidates the entry instead of serving a stale icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    modified_ms: i64,
    length: u64,
}

impl Fingerprint {
    fn read(path: &str) -> Option<Self> {
        let metadata = fs::metadata(Path::new(path)).ok()?;
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |value| value.as_millis() as i64);
        Some(Self {
            modified_ms,
            length: metadata.len(),
        })
    }
}

struct Entry {
    fingerprint: Option<Fingerprint>,
    /// `None` records that extraction failed, which stops repeated Win32 work
    /// for executables that genuinely have no readable icon.
    icon: Option<String>,
    used_at: u64,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<String, Entry>,
    clock: u64,
}

#[derive(Default)]
pub struct IconCache {
    inner: Mutex<Inner>,
}

pub enum Cached {
    Icon(String),
    Unavailable,
    Missing,
}

impl IconCache {
    pub fn get(&self, path: &str) -> Cached {
        let fingerprint = Fingerprint::read(path);
        let mut inner = self.inner.lock();
        inner.clock += 1;
        let clock = inner.clock;
        let Some(entry) = inner.entries.get_mut(path) else {
            return Cached::Missing;
        };
        if entry.fingerprint != fingerprint {
            inner.entries.remove(path);
            return Cached::Missing;
        }
        entry.used_at = clock;
        match entry.icon.as_ref() {
            Some(icon) => Cached::Icon(icon.clone()),
            None => Cached::Unavailable,
        }
    }

    pub fn insert(&self, path: &str, icon: Option<String>) {
        let fingerprint = Fingerprint::read(path);
        let mut inner = self.inner.lock();
        inner.clock += 1;
        let used_at = inner.clock;
        inner.entries.insert(
            path.to_owned(),
            Entry {
                fingerprint,
                icon,
                used_at,
            },
        );
        while inner.entries.len() > CAPACITY {
            let Some(oldest) = inner
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            inner.entries.remove(&oldest);
        }
    }
}

/// Exposed for tests and diagnostics: reports how many entries are resident.
#[cfg(test)]
impl IconCache {
    fn len(&self) -> usize {
        self.inner.lock().entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(path: &Path, contents: &[u8]) {
        let mut file = fs::File::create(path).unwrap();
        file.write_all(contents).unwrap();
    }

    #[test]
    fn cache_returns_stored_icon_and_records_failures() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.exe");
        write_file(&path, b"first");
        let key = path.to_string_lossy().to_string();
        let cache = IconCache::default();

        assert!(matches!(cache.get(&key), Cached::Missing));
        cache.insert(&key, Some("data:image/png;base64,AAAA".into()));
        assert!(matches!(cache.get(&key), Cached::Icon(_)));

        cache.insert(&key, None);
        assert!(matches!(cache.get(&key), Cached::Unavailable));
    }

    #[test]
    fn cache_invalidates_when_the_executable_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.exe");
        write_file(&path, b"first");
        let key = path.to_string_lossy().to_string();
        let cache = IconCache::default();
        cache.insert(&key, Some("stale".into()));

        write_file(&path, b"second-run-with-a-different-length");

        assert!(matches!(cache.get(&key), Cached::Missing));
    }

    #[test]
    fn cache_evicts_least_recently_used_entries() {
        let directory = tempfile::tempdir().unwrap();
        let cache = IconCache::default();
        let mut keys = Vec::new();
        for index in 0..CAPACITY + 8 {
            let path = directory.path().join(format!("app-{index}.exe"));
            write_file(&path, b"payload");
            let key = path.to_string_lossy().to_string();
            cache.insert(&key, Some(format!("icon-{index}")));
            keys.push(key);
            // Keep the first entry hot so eviction must choose a colder one.
            let _ = cache.get(&keys[0]);
        }

        assert!(cache.len() <= CAPACITY);
        assert!(matches!(cache.get(&keys[0]), Cached::Icon(_)));
    }
}
