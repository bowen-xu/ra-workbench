use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::ai::CancellationSignal;

use crate::ai::knowledge_contracts::ConceptRetrievalError;

const MAX_ACTIVE_PREVIEWS: usize = 8;

#[derive(Clone, Default)]
pub(crate) struct ConceptRetrievalManager {
    active: Arc<Mutex<HashMap<String, CancellationSignal>>>,
}

impl ConceptRetrievalManager {
    fn register(&self, request_id: &str) -> Result<CancellationSignal, ConceptRetrievalError> {
        if request_id.trim().is_empty() || request_id.len() > 160 {
            return Err(ConceptRetrievalError::InvalidInput(
                "概念网络检索请求 ID 无效".to_owned(),
            ));
        }
        let mut active = self
            .active
            .lock()
            .map_err(|_| ConceptRetrievalError::WorkerUnavailable)?;
        if active.contains_key(request_id) || active.len() >= MAX_ACTIVE_PREVIEWS {
            return Err(ConceptRetrievalError::InvalidInput(
                "概念网络检索请求已在运行或并发数量已满".to_owned(),
            ));
        }
        let signal = CancellationSignal::default();
        active.insert(request_id.to_owned(), signal.clone());
        Ok(signal)
    }

    pub(crate) fn cancel(&self, request_id: &str) -> Result<bool, ConceptRetrievalError> {
        let active = self
            .active
            .lock()
            .map_err(|_| ConceptRetrievalError::WorkerUnavailable)?;
        if let Some(signal) = active.get(request_id) {
            signal.cancel();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn release(&self, request_id: &str) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(request_id);
        }
    }
}

pub(crate) struct PreviewRegistration {
    manager: ConceptRetrievalManager,
    request_id: String,
    signal: CancellationSignal,
}
impl PreviewRegistration {
    pub(crate) fn signal(&self) -> CancellationSignal {
        self.signal.clone()
    }
}
impl Drop for PreviewRegistration {
    fn drop(&mut self) {
        self.manager.release(&self.request_id);
    }
}
impl ConceptRetrievalManager {
    pub(crate) fn register_owned(
        &self,
        request_id: &str,
    ) -> Result<PreviewRegistration, ConceptRetrievalError> {
        let signal = self.register(request_id)?;
        Ok(PreviewRegistration {
            manager: self.clone(),
            request_id: request_id.into(),
            signal,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_scoped_to_the_registered_preview() {
        let manager = ConceptRetrievalManager::default();
        let first = manager.register("preview-1").expect("first preview");
        let second = manager.register("preview-2").expect("second preview");

        assert!(manager.cancel("preview-1").expect("cancel"));
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
        manager.release("preview-1");
        assert!(!manager.cancel("missing").expect("missing preview"));
    }

    #[test]
    fn owned_registration_releases_on_return_and_unwind_without_releasing_other_slots() {
        let manager = ConceptRetrievalManager::default();
        let other = manager.register_owned("other").unwrap();
        let registration = manager.register_owned("request").unwrap();
        assert!(manager.register_owned("request").is_err());
        drop(registration);
        assert!(!manager.cancel("request").unwrap());
        let result = std::panic::catch_unwind(|| {
            let _registration = manager.register_owned("request").unwrap();
            panic!("synthetic preview panic");
        });
        assert!(result.is_err());
        assert!(!manager.cancel("request").unwrap());
        assert!(manager.cancel("other").unwrap());
        assert!(other.signal().is_cancelled());
    }
}
