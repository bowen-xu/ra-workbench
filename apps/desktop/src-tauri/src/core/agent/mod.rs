//! Native shared-agent adapters. Metadata, submission, execution and host-specific
//! source adapters live here; provider credentials remain in desktop adapters.
#[path = "context_catalog.rs"]
pub(crate) mod context_catalog;
#[path = "paper_portrait.rs"]
pub(crate) mod paper_portrait;

#[path = "authentication/mod.rs"]
pub mod authentication;
#[path = "cancellation.rs"]
pub mod cancellation;
#[path = "catalog_commands.rs"]
pub mod catalog_commands;
#[path = "child_archive.rs"]
mod child_archive;
#[path = "child_creation.rs"]
pub(crate) mod child_creation;
#[path = "child_runtime.rs"]
mod child_runtime;
#[path = "child_submission.rs"]
pub(crate) mod child_submission;
#[path = "compaction_commands.rs"]
pub mod compaction_commands;
#[cfg(unix)]
#[path = "compaction_hook_commands.rs"]
pub mod compaction_hook_commands;
#[cfg(unix)]
#[path = "compaction_hook_config.rs"]
pub mod compaction_hook_config;
#[path = "compaction_hook_factory.rs"]
pub mod compaction_hook_factory;
#[cfg(unix)]
#[path = "compaction_hook_process.rs"]
pub mod compaction_hook_process;
#[path = "compaction_hook_spill.rs"]
pub mod compaction_hook_spill;
#[path = "compile_feedback.rs"]
pub(crate) mod compile_feedback;
#[path = "compile_result_commands.rs"]
pub mod compile_result_commands;
#[path = "compile_synctex.rs"]
pub(crate) mod compile_synctex;
#[path = "composed_tools.rs"]
pub(crate) mod composed_tools;
#[path = "config.rs"]
pub mod config;
#[path = "context_session/mod.rs"]
pub mod context_session;
#[path = "context_usage.rs"]
pub mod context_usage;
#[path = "context_usage_commands.rs"]
pub mod context_usage_commands;
#[path = "execution.rs"]
pub mod execution;
#[path = "execution_profile.rs"]
pub mod execution_profile;
#[path = "export_commands.rs"]
pub mod export_commands;
#[path = "export_download.rs"]
pub mod export_download;
#[path = "git.rs"]
pub mod git;
#[path = "git_commands.rs"]
pub mod git_commands;
#[path = "git_version_commands.rs"]
pub mod git_version_commands;
#[path = "git_workspace_commands.rs"]
pub mod git_workspace_commands;
#[path = "history_wait_commands.rs"]
pub mod history_wait_commands;
#[path = "host_execution.rs"]
pub(crate) mod host_execution;
#[path = "host_instructions.rs"]
mod host_instructions;
#[path = "instruction_trust_commands.rs"]
pub mod instruction_trust_commands;
#[path = "instructions.rs"]
pub mod instructions;
#[cfg(unix)]
#[path = "mcp_commands.rs"]
pub mod mcp_commands;
#[path = "mcp_config.rs"]
pub mod mcp_config;
#[path = "mcp_http.rs"]
pub mod mcp_http;
#[cfg(unix)]
#[path = "mcp_runtime.rs"]
pub mod mcp_runtime;
#[cfg(unix)]
#[path = "mcp_stdio.rs"]
pub mod mcp_stdio;
#[cfg(unix)]
#[path = "mcp_tools.rs"]
pub mod mcp_tools;
#[path = "message_data_commands.rs"]
pub mod message_data_commands;
#[path = "model/mod.rs"]
pub mod model;
#[path = "note_application.rs"]
pub mod note_application;
#[path = "note_child_commands.rs"]
pub mod note_child_commands;
#[cfg(test)]
#[allow(dead_code)] // Archived tool implementation remains available to legacy wire tests.
#[path = "note_child_tool.rs"]
mod note_child_tool;
#[path = "note_commands.rs"]
pub mod note_commands;
#[path = "note_queue.rs"]
pub mod note_queue;
#[path = "note_queue_admission.rs"]
mod note_queue_admission;
#[path = "note_queue_preview.rs"]
pub mod note_queue_preview;
#[path = "note_queue_recovery.rs"]
pub mod note_queue_recovery;
#[path = "note_queue_start.rs"]
pub mod note_queue_start;
#[path = "note_rejection.rs"]
pub mod note_rejection;
#[path = "note_review.rs"]
pub mod note_review;
#[path = "note_scope.rs"]
mod note_scope;
#[path = "note_start.rs"]
pub mod note_start;
#[path = "note_steering.rs"]
pub mod note_steering;
#[path = "note_submission.rs"]
pub mod note_submission;
#[path = "note_tools.rs"]
mod note_tools;
#[path = "note_undo.rs"]
pub mod note_undo;
#[path = "paper_metadata.rs"]
pub(crate) mod paper_metadata;
#[path = "paper_migration.rs"]
pub mod paper_migration;
#[path = "paper_queue.rs"]
pub mod paper_queue;
#[path = "paper_recovery.rs"]
pub mod paper_recovery;
#[path = "paper_steer.rs"]
pub mod paper_steer;
#[path = "process.rs"]
pub mod process;
#[path = "queue_contract.rs"]
pub(crate) mod queue_contract;
#[path = "reading_child_commands.rs"]
pub mod reading_child_commands;
#[cfg(test)]
#[allow(dead_code)] // Archived tool implementation remains available to legacy wire tests.
#[path = "reading_child_tool.rs"]
mod reading_child_tool;
pub(crate) mod reading_context;
#[path = "reading_note_recovery.rs"]
pub mod reading_note_recovery;
#[path = "reading_note_start.rs"]
pub mod reading_note_start;
#[path = "reading_note_submission.rs"]
pub mod reading_note_submission;
#[path = "reading_sources.rs"]
pub mod reading_sources;
#[path = "reading_start.rs"]
pub mod reading_start;
#[path = "reading_submission.rs"]
pub mod reading_submission;
#[path = "repository/mod.rs"]
pub mod repository;
#[path = "resource_catalog.rs"]
pub mod resource_catalog;
#[path = "resource_management.rs"]
pub mod resource_management;
#[path = "retrieval_context.rs"]
pub(crate) mod retrieval_context;
#[path = "rollback_commands.rs"]
pub mod rollback_commands;
#[path = "service.rs"]
pub mod service;
#[path = "session_commands.rs"]
pub mod session_commands;
#[cfg(unix)]
#[path = "shell_tools.rs"]
pub mod shell_tools;
#[path = "skill_catalog_commands.rs"]
pub mod skill_catalog_commands;
#[path = "skill_document_commands.rs"]
pub(crate) mod skill_document_commands;
#[path = "skills.rs"]
pub mod skills;
#[path = "source_tools.rs"]
pub(crate) mod source_tools;
#[path = "stop_hook_process.rs"]
pub mod stop_hook_process;
#[path = "submission_response.rs"]
pub mod submission_response;
#[path = "subtask_commands.rs"]
pub mod subtask_commands;
#[path = "tasks.rs"]
pub mod tasks;
#[path = "text_tools/mod.rs"]
pub(crate) mod text_tools;
#[path = "tool_arguments.rs"]
mod tool_arguments;
#[path = "turn_commands.rs"]
pub mod turn_commands;
#[path = "turn_sink.rs"]
pub mod turn_sink;
#[path = "workspace.rs"]
pub mod workspace;
#[cfg(target_os = "macos")]
#[path = "workspace_compile.rs"]
pub(crate) mod workspace_compile;
#[path = "writing_checkpoint_contracts.rs"]
pub(crate) mod writing_checkpoint_contracts;
#[path = "writing_compile_contracts.rs"]
pub(crate) mod writing_compile_contracts;
#[path = "writing_error.rs"]
pub(crate) mod writing_error;
#[cfg(unix)]
#[path = "writing_workspace.rs"]
pub(crate) mod writing_workspace;

#[path = "writing_context.rs"]
pub(crate) mod writing_context;
mod writing_instructions;
pub(crate) mod writing_resources;
pub use writing_context::{
    WritingContextBuffer, WritingContextError, WritingContextFile, WritingContextRequest,
    WritingContextSelection,
};

#[path = "steering_contract.rs"]
mod steering_contract;
#[path = "writing_application_commands.rs"]
pub mod writing_application_commands;
#[path = "writing_child_commands.rs"]
pub mod writing_child_commands;
#[path = "writing_commands.rs"]
pub mod writing_commands;
#[path = "writing_delivery_commands.rs"]
pub mod writing_delivery_commands;
#[path = "writing_delivery_tool.rs"]
pub(crate) mod writing_delivery_tool;
#[path = "writing_discard_commands.rs"]
pub mod writing_discard_commands;
#[path = "writing_proposal_commands.rs"]
pub mod writing_proposal_commands;
#[path = "writing_queue.rs"]
pub mod writing_queue;
#[path = "writing_queue_admissions.rs"]
pub mod writing_queue_admissions;
#[path = "writing_queue_preview.rs"]
pub mod writing_queue_preview;
#[path = "writing_queued_compaction.rs"]
pub(crate) mod writing_queued_compaction;
#[path = "writing_review_commands.rs"]
pub mod writing_review_commands;
#[path = "writing_start.rs"]
pub mod writing_start;
#[path = "writing_steer.rs"]
pub mod writing_steer;
#[path = "writing_steering_recovery.rs"]
pub mod writing_steering_recovery;
#[path = "writing_submission.rs"]
pub mod writing_submission;
#[path = "writing_workspace_commands.rs"]
pub mod writing_workspace_commands;

#[path = "deletion_commands.rs"]
pub mod deletion_commands;
#[path = "evidence_images.rs"]
pub mod evidence_images;
#[path = "fork_commands.rs"]
pub mod fork_commands;
#[path = "fork_origin_commands.rs"]
pub mod fork_origin_commands;
#[path = "search_commands.rs"]
pub mod search_commands;
