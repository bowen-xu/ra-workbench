//! Bounded process-local proofs. Domains own fingerprinting and publication;
//! this container never reads documents, acquires database locks or grants writes.
use super::AppStore;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Default)]
pub(super) struct PublicationProofCache {
    entries: Mutex<VecDeque<(String, [u8; 32])>>,
}

/// A shared proof container handle; it grants no database or filesystem access.
#[derive(Clone)]
pub(crate) struct PublicationProofs {
    cache: Arc<PublicationProofCache>,
}

impl PublicationProofs {
    pub(crate) fn current(&self, id: &str, fingerprint: &[u8; 32]) -> bool {
        self.cache.contains(id, fingerprint)
    }
    pub(crate) fn invalidate(&self, id: &str) {
        self.cache.invalidate(id);
    }
    pub(crate) fn record(&self, id: &str, fingerprint: [u8; 32]) {
        self.cache.record(id, fingerprint);
    }
}

impl PublicationProofCache {
    fn lock(&self) -> MutexGuard<'_, VecDeque<(String, [u8; 32])>> {
        self.entries.lock().unwrap_or_else(|poison| {
            let mut entries = poison.into_inner();
            // Preserve fail-closed behavior: an interrupted mutation is no proof.
            // Do not clear the poison flag and later trust newly recorded entries.
            entries.clear();
            entries
        })
    }

    fn contains(&self, id: &str, fingerprint: &[u8; 32]) -> bool {
        self.lock()
            .iter()
            .any(|(key, value)| key == id && value == fingerprint)
    }

    fn invalidate(&self, id: &str) {
        self.lock().retain(|(key, _)| key != id);
    }

    fn record(&self, id: &str, fingerprint: [u8; 32]) {
        let mut entries = self.lock();
        entries.retain(|(key, _)| key != id);
        entries.push_back((id.to_owned(), fingerprint));
        while entries.len() > 64 {
            entries.pop_front();
        }
    }
}

impl AppStore {
    pub(crate) fn publication_proofs(&self) -> PublicationProofs {
        PublicationProofs {
            cache: Arc::clone(&self.bibliography_publications),
        }
    }
    #[cfg(test)]
    pub(crate) fn bibliography_publication_is_current(
        &self,
        id: &str,
        fingerprint: &[u8; 32],
    ) -> bool {
        self.bibliography_publications.contains(id, fingerprint)
    }

    #[cfg(test)]
    pub(crate) fn invalidate_bibliography_publication(&self, id: &str) {
        self.bibliography_publications.invalidate(id);
    }

    #[cfg(test)]
    pub(crate) fn record_bibliography_publication(&self, id: &str, fingerprint: [u8; 32]) {
        self.bibliography_publications.record(id, fingerprint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn clones_share_proofs_but_separate_stores_start_cold() {
        let root = tempfile::tempdir().unwrap();
        let store = AppStore::for_test(
            Connection::open_in_memory().unwrap(),
            root.path().join("objects"),
        );
        let clone = store.clone();
        store.record_bibliography_publication("same-manuscript", [1; 32]);
        assert!(clone.bibliography_publication_is_current("same-manuscript", &[1; 32]));
        clone.invalidate_bibliography_publication("same-manuscript");
        assert!(!store.bibliography_publication_is_current("same-manuscript", &[1; 32]));
        store.record_bibliography_publication("same-manuscript", [2; 32]);
        let separate = AppStore::for_test(
            Connection::open_in_memory().unwrap(),
            root.path().join("objects"),
        );
        assert!(!separate.bibliography_publication_is_current("same-manuscript", &[2; 32]));
        assert!(clone.bibliography_publication_is_current("same-manuscript", &[2; 32]));
    }

    #[test]
    fn proofs_are_bounded_and_replacing_one_preserves_other_entries() {
        let cache = PublicationProofCache::default();
        for id in 0..64 {
            cache.record(&id.to_string(), [1; 32]);
        }
        cache.record("0", [2; 32]);
        cache.record("64", [1; 32]);
        assert!(cache.contains("0", &[2; 32]));
        assert!(!cache.contains("0", &[1; 32]));
        assert!(!cache.contains("1", &[1; 32]));
        assert!(cache.contains("2", &[1; 32]));
        assert!(cache.contains("64", &[1; 32]));
        cache.invalidate("0");
        assert!(!cache.contains("0", &[2; 32]));
        assert!(cache.contains("2", &[1; 32]));
    }

    #[test]
    fn poisoned_cache_never_proves_a_publication() {
        let cache = PublicationProofCache::default();
        cache.record("before", [1; 32]);
        let panic = catch_unwind(AssertUnwindSafe(|| {
            let _guard = cache.entries.lock().unwrap();
            panic!("synthetic interrupted cache mutation");
        }));
        assert!(panic.is_err());
        assert!(!cache.contains("before", &[1; 32]));
        cache.record("after", [2; 32]);
        assert!(!cache.contains("after", &[2; 32]));
    }
}
