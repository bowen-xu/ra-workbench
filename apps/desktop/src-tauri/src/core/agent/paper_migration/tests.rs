//! Original migration regressions grouped by the behavior they preserve.
use super::*;
use crate::agent::{repository::SqliteConversationStore, service::AgentSessionService};
#[path = "tests/events.rs"]
mod events;
#[path = "tests/history.rs"]
mod history;
#[path = "tests/images.rs"]
mod images;
#[path = "tests/metadata.rs"]
mod metadata;
#[path = "tests/reconciliation.rs"]
mod reconciliation;
#[path = "tests/rename_retry.rs"]
mod rename_retry;
