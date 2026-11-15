use super::error::AnalysisError as AiError;
use crate::agent::cancellation::CancellationRegistry;
use crate::ai::CancellationSignal;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
pub(crate) struct PaperAnalysisWorkflowManager {
    registry: CancellationRegistry,
    paper_requests: Arc<Mutex<HashMap<String, String>>>,
}

pub(crate) struct PaperAnalysisWorkflowLease {
    client_request_id: String,
    manager: PaperAnalysisWorkflowManager,
    signal: CancellationSignal,
    paper_id: Option<String>,
}

impl PaperAnalysisWorkflowManager {
    pub(crate) fn register(
        &self,
        client_request_id: &str,
    ) -> Result<PaperAnalysisWorkflowLease, AiError> {
        if client_request_id.trim().is_empty() || client_request_id.len() > 200 {
            return Err(AiError::InvalidInput("论文分析请求 ID 无效".to_owned()));
        }
        let signal = self
            .registry
            .register(client_request_id, false)
            .map_err(|_| AiError::InvalidInput("该论文分析请求已启动或请求 ID 无效".to_owned()))?;
        Ok(PaperAnalysisWorkflowLease {
            client_request_id: client_request_id.to_owned(),
            manager: self.clone(),
            signal,
            paper_id: None,
        })
    }

    pub(crate) fn register_for_paper(
        &self,
        client_request_id: &str,
        paper_id: &str,
    ) -> Result<PaperAnalysisWorkflowLease, AiError> {
        if paper_id.trim().is_empty() || paper_id.len() > 200 {
            return Err(AiError::InvalidInput("论文身份无效".into()));
        }
        let mut papers = self
            .paper_requests
            .lock()
            .map_err(|_| AiError::TemporaryUnavailable)?;
        if papers.contains_key(paper_id) {
            return Err(AiError::InvalidInput(
                "本文正在提取结构化阅读，请等待完成或取消当前任务".into(),
            ));
        }
        let mut lease = self.register(client_request_id)?;
        papers.insert(paper_id.to_owned(), client_request_id.to_owned());
        lease.paper_id = Some(paper_id.to_owned());
        Ok(lease)
    }

    pub(crate) fn active_request(&self, paper_id: &str) -> Result<Option<String>, AiError> {
        Ok(self
            .paper_requests
            .lock()
            .map_err(|_| AiError::TemporaryUnavailable)?
            .get(paper_id)
            .cloned())
    }

    pub(crate) fn cancel(&self, client_request_id: &str) -> Result<bool, AiError> {
        self.registry
            .cancel(client_request_id)
            .map_err(|_| AiError::TemporaryUnavailable)
    }

    fn remove(&self, client_request_id: &str) {
        let _ = self.registry.remove(client_request_id);
    }
}

impl PaperAnalysisWorkflowLease {
    pub(crate) fn signal(&self) -> CancellationSignal {
        self.signal.clone()
    }
}

impl Drop for PaperAnalysisWorkflowLease {
    fn drop(&mut self) {
        self.manager.remove(&self.client_request_id);
        if let Some(paper_id) = &self.paper_id
            && let Ok(mut papers) = self.manager.paper_requests.lock()
            && papers.get(paper_id) == Some(&self.client_request_id)
        {
            papers.remove(paper_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_owns_registration_and_cancel_signal() {
        let manager = PaperAnalysisWorkflowManager::default();
        let lease = manager.register("request-1").expect("register");
        let other = manager.register("request-2").expect("independent request");
        assert!(manager.cancel("request-1").expect("cancel"));
        assert!(lease.signal().is_cancelled());
        assert!(!other.signal().is_cancelled());
        drop(lease);
        assert!(!manager.cancel("request-1").expect("removed"));
        drop(other);
        assert!(manager.register("request-3").is_ok());
    }

    #[test]
    fn independent_workflows_can_run_without_sharing_cancellation() {
        let manager = PaperAnalysisWorkflowManager::default();
        let first = manager.register("request-1").expect("first request");
        let second = manager.register("request-2").expect("second request");

        assert!(manager.register("request-1").is_err());
        assert!(manager.cancel("request-1").expect("cancel first"));
        assert!(first.signal().is_cancelled());
        assert!(!second.signal().is_cancelled());
    }

    #[test]
    fn paper_lease_prevents_duplicate_launch_and_exposes_the_exact_cancel_identity() {
        let manager = PaperAnalysisWorkflowManager::default();
        let first = manager.register_for_paper("request-1", "paper").unwrap();
        let other = manager.register_for_paper("request-2", "other").unwrap();
        assert!(manager.register_for_paper("request-3", "paper").is_err());
        assert_eq!(
            manager.active_request("paper").unwrap().as_deref(),
            Some("request-1")
        );
        assert!(manager.cancel("request-1").unwrap());
        assert!(first.signal().is_cancelled());
        assert!(!other.signal().is_cancelled());
        // Cancellation is pending until the original operation releases its lease.
        assert!(manager.register_for_paper("request-4", "paper").is_err());
        drop(first);
        assert!(manager.active_request("paper").unwrap().is_none());
        assert!(manager.register_for_paper("request-5", "paper").is_ok());
    }
}
