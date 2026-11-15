//! Synchronous SQLite journal primitives; call on the storage blocking executor.
//!
//! The conversation owner must serialize writes and check its live lease before
//! invoking these operations. This module does not start tasks or own connections.

#[path = "catalog.rs"]
mod catalog;
pub(crate) use catalog::WorkerRetention;
pub(crate) use catalog::binding_target_query;
pub(crate) use catalog::{AuthorizationError, authorize_rollback_in};
pub(crate) use catalog::{
    DeletionSqlError, PreparedDeletion, delete_in, deletion_replay_sql, deletion_request_hash,
};
pub(crate) use catalog::{PreparedFork, fork_in, validate_fork_request};
pub(crate) use catalog::{ValidatedArchive, archive_in, check_archive_in};
pub(crate) use catalog::{check_create_bound_in, check_create_bound_target_in, create_bound_in};
pub(crate) use catalog::{check_rename_in, rename_in};
#[cfg(test)]
#[path = "canonical_item_tests.rs"]
mod canonical_item_tests;
#[path = "change_sets.rs"]
pub mod change_sets;
#[path = "compaction_hook_audit.rs"]
pub(crate) mod compaction_hook_audit;
#[path = "compaction_hook_trust.rs"]
pub mod compaction_hook_trust;
#[path = "compactions.rs"]
pub mod compactions;
#[cfg(test)]
#[path = "completed_text_tests.rs"]
mod completed_text_tests;
#[path = "context_fragments.rs"]
pub mod context_fragments;
#[path = "context_usage.rs"]
pub(crate) mod context_usage;
#[path = "context_window_store.rs"]
pub mod context_window_store;
#[path = "context_windows.rs"]
pub mod context_windows;
#[path = "execution_workspaces.rs"]
pub mod execution_workspaces;
#[path = "git_workspaces.rs"]
pub mod git_workspaces;
#[path = "hook_spills.rs"]
pub mod hook_spills;
#[path = "input_queue.rs"]
pub mod input_queue;
#[path = "instruction_trust.rs"]
pub mod instruction_trust;
#[path = "journal.rs"]
pub(crate) mod journal;
#[path = "legacy_ai_runs.rs"]
pub mod legacy_ai_runs;
#[path = "mcp_approvals.rs"]
pub mod mcp_approvals;
#[path = "search_visibility.rs"]
pub(crate) mod search_visibility;
#[path = "sql_error.rs"]
mod sql_error;
#[path = "sqlite_store.rs"]
mod sqlite_store;
pub(crate) use sql_error::SqlRepositoryError;
#[path = "submissions.rs"]
pub(crate) mod submissions;
#[path = "subtasks.rs"]
pub mod subtasks;
#[path = "writing_start_inspection.rs"]
pub(crate) mod writing_start_inspection;

pub use catalog::{ConversationActivity, SnapshotExportError, SqliteConversationCatalog};
pub use journal::{JournalError, read_history};
#[cfg(test)]
pub use journal::{append_batch, create_conversation};
pub use sqlite_store::SqliteConversationStore;
pub(crate) use submissions::{AppImageMaterializer, SqliteSubmissionStore};

#[path = "context_policy.rs"]
pub(crate) mod context_policy;
#[path = "prefill.rs"]
pub(crate) mod prefill;
#[path = "projection.rs"]
pub mod projection;
#[path = "proposal_applications.rs"]
pub mod proposal_applications;
#[path = "proposal_undos.rs"]
pub mod proposal_undos;
#[path = "responses_state.rs"]
pub mod responses_state;
#[cfg(test)]
#[path = "session_tests.rs"]
mod session_tests;
#[cfg(test)]
#[path = "sqlite_store_tests.rs"]
mod sqlite_store_tests;
#[path = "steering.rs"]
pub mod steering;
#[path = "stop_hook_audit.rs"]
pub mod stop_hook_audit;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
