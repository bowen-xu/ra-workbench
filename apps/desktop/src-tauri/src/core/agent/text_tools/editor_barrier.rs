//! Frontend save owners flush and fence mounted editors before native text access.
use crate::storage::AppStore;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct EditorBarrier {
    state: Mutex<State>,
    events: Option<crate::runtime_events::RuntimeEvents>,
}
#[derive(Default)]
struct State {
    sessions: BTreeMap<Uuid, String>,
    pending: BTreeMap<Uuid, Pending>,
}
struct Pending {
    waiting: BTreeSet<Uuid>,
    sender: oneshot::Sender<Result<(), String>>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Flush {
    request_id: Uuid,
    session_id: Uuid,
    page_id: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Release {
    request_id: Uuid,
    pages: Vec<ReleasedPage>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReleasedPage {
    #[serde(flatten)]
    page: crate::notes::text_change_set::PageState,
    #[serde(skip_serializing_if = "Option::is_none")]
    review: Option<ReviewInstall>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewInstall {
    pub baseline: crate::notes::NoteDocument,
    pub origin: String,
    pub request_id: Option<Uuid>,
    pub add_to_history: bool,
}
pub(crate) struct Guard {
    owner: Arc<EditorBarrier>,
    request: Uuid,
    completed: bool,
}
impl Guard {
    pub(crate) fn complete(self, pages: Vec<crate::notes::text_change_set::PageState>) {
        self.complete_review(pages, BTreeMap::new());
    }
    pub(crate) fn complete_review(
        self,
        pages: Vec<crate::notes::text_change_set::PageState>,
        reviews: BTreeMap<String, ReviewInstall>,
    ) {
        let request = self.request;
        if self.try_complete_review(pages, reviews).is_err() {
            eprintln!("NOTE_EDITOR_DELIVERY_UNCONFIRMED requestId={request}");
        }
    }
    pub(crate) fn try_complete_review(
        mut self,
        pages: Vec<crate::notes::text_change_set::PageState>,
        reviews: BTreeMap<String, ReviewInstall>,
    ) -> Result<(), String> {
        if let Some(events) = &self.owner.events {
            events.emit(
                "research-note-text-release",
                Release {
                    request_id: self.request,
                    pages: pages
                        .into_iter()
                        .map(|page| ReleasedPage {
                            review: reviews.get(&page.page_id).cloned(),
                            page,
                        })
                        .collect(),
                },
            )?;
        }
        self.completed = true;
        Ok(())
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.owner.state.lock() {
            state.pending.remove(&self.request);
        }
        if !self.completed
            && let Some(events) = &self.owner.events
        {
            let _ = events.emit(
                "research-note-text-release",
                Release {
                    request_id: self.request,
                    pages: vec![],
                },
            );
        }
    }
}
impl EditorBarrier {
    pub(crate) fn publish_review_receipt(
        &self,
        pages: Vec<crate::notes::text_change_set::PageState>,
        reviews: BTreeMap<String, ReviewInstall>,
    ) -> Result<(), String> {
        if let Some(events) = &self.events {
            events.emit(
                "research-note-text-receipt",
                serde_json::json!({"pages":pages.into_iter().map(|page| {
                let review = reviews.get(&page.page_id).cloned(); ReleasedPage { page, review }
            }).collect::<Vec<_>>()}),
            )?;
        }
        Ok(())
    }
    pub(crate) fn for_app(app: tauri::AppHandle) -> Self {
        Self::for_events(crate::runtime_events::RuntimeEvents::for_app(app))
    }
    pub(crate) fn for_events(events: crate::runtime_events::RuntimeEvents) -> Self {
        Self {
            events: Some(events),
            ..Self::default()
        }
    }
    pub(crate) async fn acquire(self: &Arc<Self>, page: Option<&str>) -> Result<Guard, String> {
        self.acquire_pages(page.map(|page| BTreeSet::from([page.to_owned()])).as_ref())
            .await
    }
    pub(crate) async fn acquire_pages(
        self: &Arc<Self>,
        pages: Option<&BTreeSet<String>>,
    ) -> Result<Guard, String> {
        let request = Uuid::new_v4();
        let guard = Guard {
            owner: self.clone(),
            request,
            completed: false,
        };
        let (sender, receiver) = oneshot::channel();
        let sessions = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "editor registry unavailable")?;
            let sessions = state
                .sessions
                .iter()
                .filter(|(_, id)| pages.is_none_or(|pages| pages.contains(*id)))
                .map(|(id, page)| (*id, page.clone()))
                .collect::<Vec<_>>();
            if sessions.is_empty() {
                return Ok(guard);
            }
            state.pending.insert(
                request,
                Pending {
                    waiting: sessions.iter().map(|(id, _)| *id).collect(),
                    sender,
                },
            );
            sessions
        };
        let events = self
            .events
            .as_ref()
            .ok_or("mounted editor has no event transport")?;
        for (session_id, page_id) in sessions {
            events
                .emit(
                    "research-note-text-flush",
                    Flush {
                        request_id: request,
                        session_id,
                        page_id,
                    },
                )
                .map_err(|error| error.to_string())?;
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), receiver)
            .await
            .map_err(|_| "editor save timed out; preserve input and retry after saving")?
            .map_err(|_| "editor save response was lost")??;
        Ok(guard)
    }
    pub(crate) fn register(&self, session_id: Uuid, page_id: Option<String>) -> Result<(), String> {
        if session_id.is_nil() {
            return Err("invalid editor identity".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "editor registry unavailable")?;
        if let Some(page) = page_id {
            Uuid::parse_str(&page).map_err(|_| "invalid note identity")?;
            state.sessions.insert(session_id, page);
        } else {
            state.sessions.remove(&session_id);
        }
        Ok(())
    }
    pub(crate) fn ack(
        &self,
        request: Uuid,
        session: Uuid,
        error: Option<String>,
    ) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "editor registry unavailable")?;
        let pending = state
            .pending
            .get_mut(&request)
            .ok_or("editor request expired")?;
        if !pending.waiting.remove(&session) {
            return Err("editor response identity mismatch".into());
        }
        if error.is_some() || pending.waiting.is_empty() {
            let pending = state.pending.remove(&request).unwrap();
            let _ = pending.sender.send(error.map_or(Ok(()), Err));
        }
        Ok(())
    }
}
#[tauri::command]
pub fn agent_register_note_text_editor(
    store: tauri::State<'_, AppStore>,
    session_id: Uuid,
    page_id: Option<String>,
) -> Result<(), String> {
    store.text_editors().register(session_id, page_id)
}
#[tauri::command]
pub fn agent_ack_note_text_editor(
    store: tauri::State<'_, AppStore>,
    request_id: Uuid,
    session_id: Uuid,
    error: Option<String>,
) -> Result<(), String> {
    store.text_editors().ack(request_id, session_id, error)
}
