#[path = "core/compile_activity.rs"]
mod compile_activity;
#[path = "app/model_settings.rs"]
mod model_settings_app;
use model_settings_app::save_ai_provider_settings;
#[cfg(test)]
#[path = "core/ai_candidate_recovery_tests/optional_owner.rs"]
mod ai_candidate_optional_owner_tests;
#[path = "app/core_sources.rs"]
mod core_sources;
#[path = "core/runtime_plugins/mod.rs"]
mod runtime_plugins;
#[path = "app/runtime_plugins.rs"]
mod runtime_plugins_app;
#[cfg(all(debug_assertions, feature = "development-data"))]
#[path = "app/workbench_composition_check.rs"]
mod workbench_composition_check;
#[cfg(all(debug_assertions, feature = "development-data"))]
pub use workbench_composition_check::run as run_workbench_composition_check;
#[cfg(all(debug_assertions, feature = "development-data"))]
#[path = "app/native_preview_check.rs"]
mod native_preview_check;
#[cfg(all(debug_assertions, feature = "development-data"))]
pub use native_preview_check::run as run_native_preview_check;
#[cfg(all(debug_assertions, feature = "development-data"))]
#[path = "app/literature_catalog_check.rs"]
mod literature_catalog_check;
#[cfg(all(debug_assertions, feature = "development-data"))]
pub use literature_catalog_check::run as run_literature_catalog_check;

use runtime_plugins::literature_index::commands::{
    cancel_semantic_index_job, get_semantic_embedding_job, get_semantic_index_status,
    rebuild_semantic_index, retry_semantic_representation, run_semantic_index_reconciliation,
};
use runtime_plugins::literature_retrieval::commands::{
    cancel_concept_retrieval_preview, get_concept_retrieval_trace, preview_concept_retrieval,
};
use semantic_engine::validate_required_engine;
#[path = "app/candidate_review.rs"]
mod candidate_review_app;
#[path = "app/concept_capture.rs"]
mod concept_capture_app;
use concept_capture_app::capture_concept_usage;
#[path = "app/manual_definitions.rs"]
mod manual_definitions_app;
use manual_definitions_app::{
    list_definition_claims, list_research_object_revisions, list_runtime_definition_claims,
    list_runtime_research_object_revisions, save_definition_claim, save_runtime_definition_claim,
};
#[path = "app/concept_algorithms.rs"]
mod concept_algorithms_app;
use concept_algorithms_app::{
    analyze_concept_evolution, analyze_runtime_concept_evolution,
    analyze_runtime_shared_foundations, analyze_shared_foundations, get_concept_algorithm_run,
    get_runtime_concept_algorithm_run,
};
#[path = "app/concept_identity.rs"]
mod concept_identity_app;
use concept_identity_app::{
    apply_runtime_concept_identity, list_concept_identity_events, merge_concepts,
    preview_concept_merge, split_concepts,
};
#[path = "app/manual_relations.rs"]
mod manual_relations_app;
use manual_relations_app::{
    list_relation_assertions, list_runtime_relation_assertions, save_relation_assertion,
    save_runtime_relation_assertion, search_relation_nodes, search_runtime_relation_nodes,
};
#[path = "core/scoped_text_files/mod.rs"]
mod scoped_text_files;
#[path = "core/semantic_engine/mod.rs"]
mod semantic_engine;

#[path = "core/writing_resource_files.rs"]
mod writing_resource_files;
#[path = "core/writing_transfer_files.rs"]
mod writing_transfer_files;
use candidate_review_app::{
    decide_ai_candidate, decide_ai_paper_concept_item, decide_ai_suggestion_item,
};
#[path = "app/paper_confirmation.rs"]
mod paper_confirmation_app;
use paper_confirmation_app::{
    confirm_ai_overview_fields, confirm_ai_paper_review, undo_ai_overview_confirmation,
    undo_ai_paper_review_confirmation,
};
#[path = "app/context_catalog.rs"]
mod context_catalog_app;

use runtime_plugins::literature_analysis::{
    commands::{
        cancel_paper_analysis_workflow, get_latest_paper_analysis_workflow,
        get_paper_analysis_workflow, start_paper_analysis_workflow,
    },
    manager::PaperAnalysisWorkflowManager,
};
#[path = "core/agent/mod.rs"]
pub mod agent;

#[path = "core/ai/mod.rs"]
mod ai;

#[path = "core/application_update.rs"]
mod application_update;
#[path = "app/browser_capture.rs"]
mod browser_capture_app;
#[path = "core/browser_connector/mod.rs"]
mod browser_connector;

#[path = "core/content/mod.rs"]
mod content;
#[path = "core/data_interface/mod.rs"]
mod data_interface;
#[path = "app/desktop_lifecycle.rs"]
mod desktop_lifecycle;
#[path = "core/dictionary.rs"]
mod dictionary;
#[path = "core/docling/mod.rs"]
mod docling;
#[path = "core/doi.rs"]
mod doi;

#[path = "core/export_file.rs"]
mod export_file;
#[path = "core/hbs_commands.rs"]
mod hbs_commands;
#[path = "core/hbs_storage.rs"]
mod hbs_storage;

#[cfg(all(debug_assertions, feature = "development-data", unix))]
#[path = "core/native_acceptance_pause.rs"]
mod native_acceptance_pause;
#[path = "core/note_export/mod.rs"]
mod note_export;
#[path = "core/note_recovery_export.rs"]
mod note_recovery_export;
#[path = "core/notes/mod.rs"]
mod notes;

#[path = "app/paper_history.rs"]
mod paper_history_app;
#[path = "app/paper_portrait_history.rs"]
mod paper_portrait_history;

#[path = "core/project.rs"]
mod project;
#[path = "app/research_export.rs"]
mod research_export;
#[path = "core/runtime_events.rs"]
mod runtime_events;
#[path = "core/storage.rs"]
mod storage;
#[path = "core/storage_backup.rs"]
mod storage_backup;
#[path = "core/storage_environment.rs"]
mod storage_environment;
#[path = "core/storage_integrity.rs"]
mod storage_integrity;
#[cfg(test)]
#[path = "core/storage_migration_tests.rs"]
mod storage_migration_tests;
#[path = "core/storage_migrations.rs"]
mod storage_migrations;
#[cfg(test)]
#[path = "core/storage_p0_migration_tests.rs"]
mod storage_p0_migration_tests;
#[path = "core/storage_query.rs"]
mod storage_query;
#[path = "core/structured_resources/mod.rs"]
mod structured_resources;

#[path = "core/system_documents/mod.rs"]
mod system_documents;

use ai::{
    AiRunManager, CodexAppServerManager, JsonCredentialStore, cancel_ai_run, cancel_chatgpt_login,
    create_ai_candidate_summary_version, delete_ai_credential,
    discard_other_ai_candidate_summary_versions, fetch_llm_provider_models,
    get_ai_credential_summary, get_ai_run, get_chatgpt_account_status, get_llm_configuration,
    list_ai_candidates, list_ai_provider_overviews, list_ai_runs, logout_chatgpt, preview_ai_task,
    record_ai_attention_feedback, record_ai_citation_assessment, record_ai_evidence_inspection,
    revise_ai_candidate, save_llm_model_routes, save_llm_provider_config,
    select_ai_candidate_summary_version, set_ai_credential, start_ai_run, start_chatgpt_login,
    test_ai_provider,
};
#[cfg(feature = "development-data")]
pub use ai::{CalibrationPricing, CalibrationRunOptions, run_calibration};
use content::{
    ContentGcManager, cancel_content_gc, get_content_gc_status, get_content_storage_report,
    scan_content_storage, schedule_startup_maintenance, start_content_gc,
};
use desktop_lifecycle::{confirm_app_quit, handle_run_event};
use docling::{
    DoclingServiceManager, cancel_docling_parse_task, check_docling_model_update,
    connect_docling_service, delete_docling_models, disconnect_docling_service,
    download_docling_models, get_docling_model_status, get_docling_model_storage,
    get_docling_page_overlay, get_docling_parse_run, get_docling_parse_task,
    get_docling_service_status, get_docling_settings, list_docling_parse_runs,
    parse_attachment_with_docling, ping_docling_service, save_docling_settings,
    schedule_startup_service, start_docling_parse_task,
};

use notes::query_note_page_catalog;
use notes::{
    NoteAssetManager, NoteIndexManager, NoteTransferManager, apply_note_metadata_batch,
    cancel_note_asset_import, cancel_note_export, cancel_note_index_rebuild,
    commit_note_navigation, create_literature_note, create_note_page, create_reading_note,
    duplicate_note_page, ensure_note_tag, export_notes, get_note_index_state, get_reading_note,
    import_note_asset, list_literature_notes, list_note_backlinks, list_note_favorites,
    list_note_links, list_note_page_metadata, list_note_pages, list_note_revisions,
    list_note_source_labels, list_note_tags, list_reading_notes, load_note_document,
    load_note_index_documents, load_note_revision, move_note_page, permanently_delete_note_page,
    permanently_delete_reading_note, read_note_asset, rebuild_note_index, remove_note_link,
    reorder_note_favorites, replay_note_metadata_batch, resolve_note_conflict,
    restore_note_revision, save_note_document, search_notes, set_note_page_status,
    set_note_revision_pinned, set_reading_note_status, update_note_page, update_reading_note,
    upsert_note_link,
};
use paper_history_app::{
    delete_paper_ai_reading_session, get_paper_ai_reading_session,
    get_paper_ai_reading_turn_debug_data, list_paper_ai_reading_sessions,
    prepare_paper_reading_bootstrap,
};
use paper_portrait_history::get_latest_paper_portrait_run;
use project::{create_project, list_projects};
use research_export::{
    ResearchExportManager, cancel_research_export, cancel_runtime_research_export,
    export_research_knowledge, export_runtime_research_knowledge,
};
use storage::AppStore;

use tauri::Manager;

#[cfg(all(debug_assertions, feature = "development-data"))]
#[path = "app/api_test_host.rs"]
mod api_test_host;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    #[cfg(all(debug_assertions, feature = "development-data"))]
    let context = api_test_host::configure_context(context)
        .expect("failed to configure Workbench API test host");
    runtime_plugins_app::configure(tauri::Builder::default())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                agent::history_wait_commands::close_window(window);
            }
        })
        .setup(|app| {
            #[cfg(all(debug_assertions, feature = "development-data"))]
            api_test_host::create_window(app)?;
            #[cfg(target_os = "macos")]
            {
                desktop_lifecycle::install_termination_guard(app.handle())?;
                desktop_lifecycle::install_quit_menu(app.handle())?;
            }
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;
            app.manage(application_update::ApplicationUpdate::default());

            runtime_plugins_app::attach_bundled_packages(app.handle())?;
            validate_required_engine().map_err(|error| error.to_string())?;
            let store = AppStore::open(
                app.handle(),
                runtime_plugins_app::attach_note_trace(app.handle(), core_sources::sources()),
            )
                .map_err(|error| error.to_string())?;
            let store = runtime_plugins_app::attach_writing_context(app.handle(), store);
            app.manage(browser_capture_app::start(app.handle(), store.clone()));
            agent::skills::bundled::install_defaults(store.agent_configuration_directory())
                .map_err(|error| format!("无法初始化应用内置技能：{error}"))?;
            agent::repository::steering::SqliteSteeringStore::reconcile_startup(&store)
                .map_err(|error| format!("Agent steering startup reconciliation failed: {error}"))?;
            if let Ok(mut db) = store.connection()
                && let Err(error) = agent::repository::git_workspaces::startup::mark_missing_on_startup(&mut db)
            {
                eprintln!("Git worktree startup reconciliation failed: {error}");
            }
            if let Ok(mut db) = store.connection()
                && let Err(error) = agent::repository::subtasks::recover_on_startup(&mut db)
            {
                eprintln!("Agent child-task startup reconciliation failed: {error}");
            }
            let _ = agent::compile_result_commands::cleanup_expired_agent_compile_results(&store);
            let agent_sessions = std::sync::Arc::new(
                agent::service::AgentSessionService::from_app_store(&store)
                    .map_err(|error| error.to_string())?,
            );
            let credential_store =
                JsonCredentialStore::from_app(app.handle()).map_err(|error| error.to_string())?;
            let codex_manager =
                CodexAppServerManager::from_app(app.handle()).map_err(|error| error.to_string())?;
            let agent_chatgpt = std::sync::Arc::new(
                agent::authentication::AgentChatGptAuthentication::new(codex_manager.clone()),
            );
            let agent_models =
                ai::AgentModelResolver::from_app_store(&store, credential_store.clone())
                    .with_chatgpt(agent_chatgpt.clone())
                    .with_context_metadata(app.handle());
            app.manage(ai::word_explanation::WordExplanationManager::default());
            #[cfg(unix)]
            let hook_settings = std::sync::Arc::new(
                agent::compaction_hook_commands::HookConsentService::from_app_store(&store),
            );
            #[cfg(target_os = "macos")]
            let agent_models = agent_models.with_native_compaction_hooks(hook_settings.clone());
            #[cfg(unix)]
            app.manage(hook_settings);
            let docling_manager =
                DoclingServiceManager::from_app(app.handle()).map_err(|error| error.to_string())?;
            store.docling.set(std::sync::Arc::new(docling::preparation::DoclingPreparation::new(docling_manager.clone())))
                .map_err(|_| "Docling preparation already initialized")?;
            let startup_store = store.clone();
            let startup_docling_manager = docling_manager.clone();


            if let Err(error) = runtime_plugins_app::purge_expired_trash_on_startup(app.handle(), &store) {
                eprintln!("回收站到期清理失败：{error}");
            }
            let content_manager = ContentGcManager::default();
            schedule_startup_maintenance(store.clone(), content_manager.clone());
            #[cfg(unix)]
            agent::writing_submission::workspace::schedule_object_recovery(store.clone());
            app.manage(notes::NoteExportAssetReader::for_store(&store));

            app.manage(store);
            app.manage(agent_sessions);
            app.manage(
                agent::repository::SqliteConversationCatalog::from_app_store(&startup_store),
            );
            app.manage(
                agent::writing_submission::WritingSubmissionService::from_app_store(&startup_store)
                    .with_model_resolver(agent_models.clone()),
            );
            app.manage(agent_models);
            app.manage(agent_chatgpt);
            app.manage(ai::EmbeddingAccess::new(credential_store.clone()));
            app.manage(credential_store);
            app.manage(codex_manager);
            app.manage(docling_manager);
            app.manage(AiRunManager::default());
            app.manage(PaperAnalysisWorkflowManager::default());
            app.manage(runtime_plugins::literature_index::manager::SemanticIndexManager::default());
            app.manage(runtime_plugins::literature_retrieval::manager::ConceptRetrievalManager::default());
            app.manage(content_manager);
            app.manage(compile_activity::CompileManager::default());
            app.manage(NoteIndexManager::default());
            app.manage(NoteTransferManager::default());
            app.manage(NoteAssetManager::default());
            app.manage(note_export::NoteExportJobs::default());
            app.manage(ResearchExportManager::default());
            schedule_startup_service(startup_store, startup_docling_manager);
            #[cfg(all(debug_assertions, feature = "development-data"))]
            api_test_host::start(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            runtime_plugins::list_runtime_plugin_packages,
            runtime_plugins::native::preview_runtime_manuscript_import,
            runtime_plugins::native::preview_runtime_writing_project_import,
            runtime_plugins::writing_projects::list_runtime_manuscripts,
            runtime_plugins::writing_projects::citation_metadata::list_runtime_manuscript_citation_bindings,
            runtime_plugins::writing_projects::citation_metadata::get_runtime_manuscript_bibliography_status,
            runtime_plugins::writing_projects::files::commands::list_runtime_manuscript_files,
            runtime_plugins::writing_projects::files::bibliography::resource_import::import_runtime_manuscript_resources,
            runtime_plugins::writing_projects::files::bibliography::project_transfer::commands::import_runtime_manuscript_project,
            runtime_plugins::writing_projects::files::bibliography::project_transfer::commands::duplicate_runtime_manuscript,
            runtime_plugins::writing_projects::files::transfer::export_runtime_manuscript_source,
            runtime_plugins::writing_projects::compile::export::export_runtime_manuscript_pdf,
            runtime_plugins::writing_projects::files::note_assets::commands::import_runtime_writing_note_asset,
            runtime_plugins::writing_projects::files::note_assets::commands::read_runtime_writing_note_asset,
            runtime_plugins::writing_projects::files::note_assets::commands::export_runtime_writing_note_asset,
            runtime_plugins::writing_projects::files::bibliography::list_runtime_manuscript_bibliography_entries,
            runtime_plugins::writing_projects::files::bibliography::sync::sync_runtime_manuscript_bibliography,
            runtime_plugins::writing_projects::files::bibliography::rekey::rekey_runtime_manuscript_citations,
            runtime_plugins::writing_projects::files::bibliography::binding::bind_runtime_manuscript_citation,
            runtime_plugins::writing_projects::files::bibliography::search::search_runtime_manuscript_citations,
            runtime_plugins::writing_projects::files::bibliography::mutation::update_runtime_manuscript_bibliography_entry,
            runtime_plugins::writing_projects::files::bibliography::mutation::link_runtime_manuscript_bibliography_entry,
            runtime_plugins::writing_projects::files::bibliography::mutation::remove_unused_runtime_imported_bibliography_entries,
            runtime_plugins::writing_projects::files::bibliography::recovery::restore_runtime_manuscript_bibliography,
            runtime_plugins::writing_projects::files::commands::read_runtime_manuscript_file,
            runtime_plugins::writing_projects::files::commands::save_runtime_manuscript_file,
            runtime_plugins::writing_projects::files::save::create_runtime_manuscript_file,
            runtime_plugins::writing_projects::files::changes::rename_runtime_manuscript_file,
            runtime_plugins::writing_projects::files::changes::delete_runtime_manuscript_file,
            runtime_plugins::writing_projects::files::directories::commands::list_runtime_manuscript_directories,
            runtime_plugins::writing_projects::files::directories::commands::create_runtime_manuscript_directory,
            runtime_plugins::writing_projects::files::directories::commands::rename_runtime_manuscript_directory,
            runtime_plugins::writing_projects::files::deletion::delete_runtime_manuscript,
            runtime_plugins::writing_projects::files::catalog_preparation::prepare_runtime_writing_note_catalog,
            runtime_plugins::writing_projects::files::catalog_query::query_runtime_writing_note_catalog,
            runtime_plugins::writing_projects::files::checkpoints::save_runtime_writing_document_checkpoint,
            runtime_plugins::writing_projects::files::checkpoints::list_runtime_writing_document_checkpoints,
            runtime_plugins::writing_projects::files::directories::commands::delete_runtime_manuscript_directory,
            runtime_plugins::writing_projects::creation::create_runtime_manuscript,
            runtime_plugins::writing_projects::status::archive_runtime_manuscript,
            runtime_plugins::writing_projects::status::restore_runtime_manuscript,
            runtime_plugins::writing_projects::metadata::update_runtime_manuscript,
            runtime_plugins::writing_projects::recovery::list_runtime_manuscript_recovery_drafts,
            runtime_plugins::writing_projects::recovery::save_runtime_manuscript_recovery_draft,
            runtime_plugins::writing_projects::recovery::clear_runtime_manuscript_recovery_draft,
            runtime_plugins::writing_projects::sessions::get_runtime_writing_session,
            runtime_plugins::writing_projects::sessions::save_runtime_writing_session,
            runtime_plugins::writing_projects::get_runtime_manuscript,
            runtime_plugins::writing_projects::compile::read::detect_runtime_latex_toolchain,
            runtime_plugins::writing_projects::compile::synctex::forward_runtime_manuscript_synctex,
            runtime_plugins::writing_projects::compile::synctex::inverse_runtime_manuscript_synctex,
            runtime_plugins::writing_projects::compile::start::start_runtime_manuscript_compile,
            runtime_plugins::writing_projects::compile::artifacts::is_runtime_manuscript_compile_current,
            runtime_plugins::writing_projects::compile::artifacts::read_runtime_manuscript_compile_pdf,
            runtime_plugins::writing_projects::compile::artifacts::cancel_runtime_manuscript_compile,
            runtime_plugins::writing_projects::compile::read::get_runtime_manuscript_compile_run,
            runtime_plugins::writing_projects::compile::read::get_latest_runtime_manuscript_compile_run,
            runtime_plugins::writing_projects::compile::read::get_latest_pdf_runtime_manuscript_compile_run,
            runtime_plugins::literature_catalog::commands::list_runtime_literature,
            runtime_plugins::literature_catalog::commands::list_runtime_trashed_literature,
            runtime_plugins::literature_catalog::commands::get_runtime_literature_item,
            runtime_plugins::literature_catalog::commands::save_runtime_literature_item,
            runtime_plugins::literature_duplicates::commands::list_runtime_duplicate_candidates,
            runtime_plugins::literature_duplicates::commands::list_runtime_merge_operations,
            runtime_plugins::literature_duplicate_undo::commands::undo_runtime_merge_operation,
            runtime_plugins::literature_duplicate_merge::commands::merge_runtime_duplicate_candidate,
            runtime_plugins::literature_lifecycle::commands::apply_runtime_literature_lifecycle,
            runtime_plugins::literature_metadata::commands::apply_runtime_literature_metadata,
            runtime_plugins::literature_import_jobs::commands::apply_runtime_literature_import_jobs,
            runtime_plugins::literature_evidence::commands::list_runtime_evidence_claims,
            runtime_plugins::literature_evidence::commands::list_runtime_evidence_sources,
            runtime_plugins::literature_evidence::commands::list_runtime_claim_evidence,
            runtime_plugins::literature_matrix::commands::list_runtime_evidence_matrices,
            runtime_plugins::literature_claims::commands::save_runtime_evidence_claim,
            runtime_plugins::literature_claims::commands::link_runtime_claim_to_anchor,
            runtime_plugins::literature_claims::commands::create_runtime_claim_with_evidence,
            runtime_plugins::literature_derived_view::commands::list_runtime_derived_research_views,
            runtime_plugins::literature_derived_view::commands::save_runtime_derived_research_view,
            runtime_plugins::literature_matrix::commands::list_runtime_evidence_matrix_cells,
            runtime_plugins::literature_matrix::commands::save_runtime_evidence_matrix,
            runtime_plugins::literature_matrix::commands::save_runtime_evidence_matrix_cell,
            runtime_plugins::literature_reference_records::commands::list_runtime_reference_records,
            runtime_plugins::literature_reference_records::commands::save_runtime_reference_record,
            runtime_plugins::literature_concepts::commands::search_runtime_research_concepts,
            runtime_plugins::literature_concepts::commands::save_runtime_research_concept,
            runtime_plugins::literature_concepts::commands::capture_runtime_research_concept_usage,
            runtime_plugins::literature_reading_units::commands::save_runtime_structured_reading_unit,
            runtime_plugins::literature_reading_units::commands::list_runtime_structured_reading_units,
            runtime_plugins::literature_attachments::commands::attach_runtime_pdf_path,
            runtime_plugins::literature_attachments::commands::rename_runtime_literature_attachment,
            runtime_plugins::literature_attachments::commands::remove_runtime_literature_attachment,
            runtime_plugins::literature_attachments::commands::read_runtime_attachment,
            runtime_plugins::literature_attachments::commands::reveal_runtime_attachments_in_file_manager,
            runtime_plugins::literature_attachments::commands::save_runtime_reading_state,
            runtime_plugins::literature_attachments::commands::get_runtime_reading_state,
            runtime_plugins::literature_attachments::commands::runtime_attachment_is_indexed,
            runtime_plugins::literature_attachments::commands::index_runtime_attachment_text,
            runtime_plugins::literature_pdf_import::commands::start_runtime_pdf_import,
            runtime_plugins::literature_doi_commands::preview_runtime_doi_metadata,
            runtime_plugins::literature_duplicates::commands::decide_runtime_duplicate_candidate,
            runtime_plugins::literature_nal_query::commands::read_runtime_nal_query_collections,
            runtime_plugins::literature_nal_bootstrap::commands::read_runtime_nal_network,
            runtime_plugins::literature_nal_annotation::commands::ensure_runtime_nal_annotation,
            runtime_plugins::literature_nal_mutation::commands::mutate_runtime_nal_network,
            runtime_plugins::literature_nal_query::commands::save_runtime_nal_query_collections,
            hbs_commands::hbs_read_document,
            hbs_commands::hbs_commit_document,
            notes::draft::publish_note_draft,
            hbs_commands::hbs_read_preservation,
            hbs_commands::hbs_save_preservation,
            application_update::application_update_status,
            application_update::application_update_check,
            application_update::application_update_download,
            application_update::application_update_pause,
            application_update::application_update_install,
            structured_resources::structured_list_resources,
            structured_resources::structured_get_resource,
            structured_resources::structured_get_resource_version,
            structured_resources::structured_get_resource_sources,
            structured_resources::structured_create_template,
            structured_resources::structured_write_resource,
            structured_resources::structured_patch_resource,
            structured_resources::structured_review_resource,
            structured_resources::structured_restore_resource_history,
            structured_resources::structured_bind_template,
            structured_resources::structured_import_legacy,
            structured_resources::structured_resource_import_startup,
            agent::context_session::configuration::agent_read_context_configuration,
            agent::context_session::configuration::agent_save_context_configuration,
            agent::context_session::configuration::agent_save_context_configuration_proven,
            agent::context_session::configuration::agent_read_context_configuration_save_status,
            agent::context_session::configuration::agent_seed_context_configuration_proven,
            agent::context_session::configuration::agent_read_context_configuration_seed_status,
            agent::context_session::configuration::agent_read_context_configuration_environment,
            browser_connector::get_browser_connector_status,
            agent::text_tools::decisions::agent_read_text_patch_proposal,
            agent::text_tools::decisions::agent_note_text_patch_conversations,
            agent::text_tools::decisions::agent_decide_text_patch,
            agent::text_tools::note_review::agent_resolve_note_text_patch,
            agent::text_tools::editor_barrier::agent_register_note_text_editor,
            agent::text_tools::editor_barrier::agent_ack_note_text_editor,
            ai::word_explanation::explain_dictionary_word,
            ai::word_explanation::follow_up_dictionary_word,
            ai::word_explanation_repository::save_word_explanation,
            ai::word_explanation_repository::list_word_explanations,
            ai::word_explanation_repository::delete_word_explanation,
            ai::word_explanation::cancel_word_explanation,
            dictionary::read_dictionary_asset,
            agent::catalog_commands::agent_list_conversations,
            agent::catalog_commands::agent_read_conversation_activity,
            agent::catalog_commands::agent_set_conversation_archived,
            agent::catalog_commands::agent_read_conversation_archive_status,
            agent::catalog_commands::agent_read_conversation_metadata,
            agent::export_commands::agent_export_conversation_snapshot,
            agent::message_data_commands::agent_read_message_data,
            agent::evidence_images::agent_pin_evidence_image,
            agent::evidence_images::agent_read_evidence_image,
            agent::evidence_images::agent_release_evidence_image,
            agent::evidence_images::agent_reconcile_evidence_drafts,
            agent::export_download::agent_download_conversation,
            agent::session_commands::agent_create_conversation,
            agent::context_session::agent_search_context_resources,
            agent::context_session::agent_search_context_catalog,
            agent::context_session::agent_recommend_context,
            agent::context_session::agent_list_context_conversations,
            agent::context_session::workspace::agent_list_context_directories,
            agent::context_session::workspace::agent_read_context_directory,
            agent::context_session::workspace::agent_set_context_directory,
            agent::context_session::agent_prepare_context_submission,
            agent::context_session::prepare_status::agent_prepare_context_submission_proven,
            agent::context_session::prepare_status::agent_read_context_submission_prepare_status,
            agent::context_session::start::agent_start_context_submission_guarded,
            agent::context_session::start::agent_read_context_start_status,
            agent::context_session::queue::agent_read_context_input_queue,
            agent::context_session::queue::agent_read_context_input_queue_edit_status,
            agent::context_session::queue::agent_edit_context_input_queue,
            agent::context_session::queue::agent_read_pending_context_queue_preview,
            agent::context_session::agent_restore_context_submission,
            agent::context_session::start::agent_start_context_submission,
            agent::context_session::steering::agent_steer_context_turn,
            agent::context_session::steering::agent_prepare_steering_handoff,
            agent::context_session::steering::agent_finish_steering_handoff,
            agent::context_session::steering::agent_release_steering_handoff,
            agent::context_session::steering::agent_cancel_unregistered_steering_handoff,
            agent::context_session::steering::agent_read_steering_handoffs,
            agent::context_session::steering::agent_read_steering_handoff_resolution,
            agent::session_commands::agent_open_conversation,
            agent::session_commands::agent_read_history,
            agent::session_commands::agent_wait_for_history,
            agent::history_wait_commands::agent_create_history_wait,
            agent::history_wait_commands::agent_wait_for_history_cancellable,
            agent::history_wait_commands::agent_cancel_history_wait,
            agent::skill_catalog_commands::agent_list_application_skills,
            agent::skill_document_commands::agent_read_skill_document,
            system_documents::commands::system_documents_location,
            system_documents::commands::text_document_export,
            note_recovery_export::export_note_recovery_file,
            note_export::note_document_export_capabilities,
            note_export::begin_note_document_export,
            note_export::cancel_note_document_export,
            note_export::release_note_document_export,
            note_export::run_note_document_export,
            system_documents::commands::system_documents_list,
            system_documents::commands::system_documents_read,
            system_documents::commands::system_documents_write,
            system_documents::commands::system_documents_pending,
            system_documents::commands::system_documents_decide,
            agent::resource_catalog::agent_list_host_resources,
            agent::resource_management::agent_scan_skill_import,
            agent::resource_management::agent_install_skill_import,
            agent::resource_management::agent_remove_skill,
            agent::resource_management::agent_cancel_skill_import,
            agent::resource_management::agent_read_mcp_configuration,
            agent::resource_management::agent_save_mcp_configuration,
            agent::resource_management::agent_set_mcp_credential,
            agent::host_execution::agent_list_execution_hosts,
            agent::host_execution::agent_set_host_shell_authorization,
            agent::skill_catalog_commands::agent_set_application_skill_enabled,
            agent::subtask_commands::agent_list_child_tasks,
            agent::subtask_commands::agent_reconcile_child_task,
            agent::writing_child_commands::agent_spawn_writing_child,
            agent::turn_commands::agent_interrupt_turn,
            agent::turn_commands::agent_observe_task,
            #[cfg(unix)]
            agent::mcp_commands::agent_install_mcp_runtime,
            agent::mcp_commands::agent_list_mcp_servers,
            agent::mcp_commands::agent_revoke_mcp_server,
            agent::compile_result_commands::read_agent_compile_pdf,
            agent::compile_result_commands::read_agent_compile_synctex,
            agent::compile_result_commands::forward_agent_compile_synctex,
            agent::compile_result_commands::inverse_agent_compile_synctex,
            agent::git_commands::agent_create_manuscript_git_version,
            agent::git_commands::agent_list_manuscript_git_versions,
            agent::git_commands::agent_initialize_manuscript_git_repository,
            agent::git_commands::agent_list_manuscript_git_branches,
            agent::git_commands::agent_create_manuscript_git_branch,
            agent::git_commands::agent_delete_manuscript_git_branch,
            agent::git_commands::agent_diff_manuscript_git_versions,
            agent::git_commands::agent_restore_manuscript_git_version,
            agent::git_commands::agent_latest_manuscript_git_restore_scope,
            agent::git_workspace_commands::agent_create_git_worktree,
            agent::git_workspace_commands::agent_list_git_worktrees,
            agent::git_workspace_commands::agent_remove_git_worktree,
            agent::git_version_commands::agent_read_manuscript_git_version_file,
            agent::instruction_trust_commands::agent_read_manuscript_instruction_trust,
            agent::instruction_trust_commands::agent_update_manuscript_instruction_trust,
            #[cfg(unix)]
            agent::compaction_hook_commands::agent_review_compaction_hooks,
            #[cfg(unix)]
            agent::compaction_hook_commands::agent_approve_compaction_hook,
            #[cfg(unix)]
            agent::compaction_hook_commands::agent_revoke_compaction_hook,
            agent::writing_commands::agent_prepare_writing_submission,
            agent::writing_commands::agent_read_writing_submission_prepare_status,
            agent::writing_delivery_commands::agent_list_writing_file_deliveries,
            agent::writing_delivery_commands::agent_acknowledge_writing_file_delivery,
            agent::writing_delivery_tool::agent_report_writing_file_delivery,
            agent::writing_delivery_tool::agent_report_writing_successor_workspace,
            agent::writing_commands::agent_restore_writing_submission,
            agent::writing_workspace_commands::agent_prepare_writing_workspace,
            agent::writing_workspace_commands::agent_prepare_next_writing_workspace,
            agent::writing_workspace_commands::agent_recover_next_writing_workspace,
            agent::writing_workspace_commands::agent_restore_writing_workspace,
            agent::writing_workspace_commands::agent_approve_writing_workspace,
            agent::writing_workspace_commands::agent_revoke_writing_workspace,
            agent::writing_proposal_commands::agent_publish_writing_proposal,
            agent::writing_application_commands::agent_writing_proposal_application,
            agent::writing_application_commands::restoration::agent_writing_editor_restoration,
            agent::writing_application_commands::undo_restoration::agent_writing_undo_editor_restoration,
            agent::writing_application_commands::undo::agent_writing_proposal_undo,
            agent::writing_application_commands::undo::agent_find_writing_proposal_undo,
            agent::writing_application_commands::undo::discovery::agent_list_pending_writing_undos,
            agent::writing_application_commands::discovery::agent_list_pending_writing_applications,
            agent::writing_application_commands::agent_find_writing_proposal_application,
            agent::writing_proposal_commands::agent_latest_writing_proposal,
            agent::writing_proposal_commands::agent_reject_writing_proposal,
            agent::writing_proposal_commands::agent_review_writing_proposal,
            agent::writing_proposal_commands::agent_review_writing_proposal_file,
            agent::writing_review_commands::agent_review_writing_workspace,
            agent::writing_review_commands::agent_review_writing_workspace_binding,
            agent::writing_review_commands::agent_list_writing_workspace_reviews,
            agent::writing_discard_commands::agent_find_writing_workspace_discard,
            agent::writing_discard_commands::actions::agent_preview_writing_workspace_discard,
            agent::writing_discard_commands::actions::agent_discard_writing_workspace,
            agent::writing_review_commands::agent_review_writing_workspace_file,
            agent::writing_start::agent_start_writing_submission,
            agent::writing_start::agent_read_writing_start_status,
            agent::compaction_commands::agent_read_writing_compaction_start_status,
            agent::writing_child_commands::agent_resume_writing_child,
            agent::reading_child_commands::agent_resume_paper_child,
            agent::note_child_commands::agent_resume_note_child,
            agent::note_child_commands::agent_resume_reading_note_child,
            agent::writing_steer::agent_steer_writing_turn,
            agent::writing_steering_recovery::agent_read_writing_steering_recovery,
            agent::writing_queue::agent_read_writing_input_queue,
            agent::writing_queue::agent_read_writing_input_queue_edit_status,
            agent::writing_queue_admissions::agent_list_unstarted_writing_queue_admissions,
            agent::writing_queue::agent_edit_writing_input_queue,
            agent::writing_queue_preview::agent_prepare_writing_queue_preview,
            agent::writing_queue_preview::agent_read_writing_queue_preview_prepare_status,
            agent::writing_queued_compaction::agent_prepare_writing_queued_compaction,
            agent::writing_queued_compaction::agent_read_writing_queued_compaction,
            agent::writing_queued_compaction::agent_settle_writing_queued_compaction,
            agent::writing_queue_preview::agent_read_pending_writing_queue_preview,
            agent::writing_queue_preview::agent_cancel_writing_queue_preview,
            agent::writing_queue_preview::agent_read_writing_queue_preview_cancel_status,
            agent::writing_start::agent_start_writing_queued_submission,
            agent::writing_start::agent_read_writing_queued_start_status,
            agent::catalog_commands::agent_rename_conversation,
            agent::catalog_commands::agent_read_conversation_rename_status,
            agent::catalog_commands::agent_create_bound_conversation,
            agent::catalog_commands::agent_read_bound_conversation_creation_status,
            agent::child_creation::status::agent_read_child_creation_status,
            agent::paper_migration::agent_migrate_paper_reading_history,
            agent::paper_queue::agent_read_paper_input_queue,
            agent::paper_queue::agent_read_paper_input_queue_edit_status,
            agent::paper_queue::agent_edit_paper_input_queue,
            agent::paper_steer::agent_steer_paper_turn,
            agent::search_commands::agent_search_conversations,
            agent::fork_commands::agent_fork_conversation,
            agent::fork_commands::agent_read_conversation_fork_status,
            agent::rollback_commands::agent_rollback_conversation,
            agent::rollback_commands::agent_read_rollback_authorization_status,
            agent::fork_origin_commands::agent_read_conversation_fork_origin,
            agent::deletion_commands::agent_delete_conversation,
            agent::deletion_commands::agent_delete_conversations_batch,
            agent::deletion_commands::agent_read_conversation_deletion_status,





            list_projects,
            export_research_knowledge,
            export_runtime_research_knowledge,
            cancel_runtime_research_export,
            cancel_research_export,
            get_content_storage_report,
            get_content_gc_status,
            scan_content_storage,
            start_content_gc,
            cancel_content_gc,
            get_docling_settings,
            get_docling_model_storage,
            delete_docling_models,
            get_docling_model_status,
            check_docling_model_update,
            download_docling_models,
            save_docling_settings,
            get_docling_service_status,
            ping_docling_service,
            connect_docling_service,
            disconnect_docling_service,
            parse_attachment_with_docling,
            start_docling_parse_task, get_docling_parse_task, cancel_docling_parse_task,
            list_docling_parse_runs,
            get_docling_parse_run,
            get_docling_page_overlay,
            get_ai_credential_summary,
            get_chatgpt_account_status,
            start_chatgpt_login,
            cancel_chatgpt_login,
            logout_chatgpt,
            set_ai_credential,
            delete_ai_credential,
            get_llm_configuration,
            save_llm_provider_config,
            save_llm_model_routes,
            fetch_llm_provider_models,
            ai::codex_model_catalog::commands::get_codex_catalog_versions,
            ai::codex_model_catalog::commands::select_codex_catalog_version,
            list_ai_provider_overviews,
            save_ai_provider_settings,
            test_ai_provider,
            preview_ai_task,
            start_ai_run,
            ai::paper_context::prepare::prepare_paper_ai_reading_context,
            prepare_paper_reading_bootstrap,
            start_paper_analysis_workflow,
            cancel_paper_analysis_workflow,
            get_paper_analysis_workflow,
            get_latest_paper_analysis_workflow,
            list_paper_ai_reading_sessions,
            get_paper_ai_reading_session,
            get_paper_ai_reading_turn_debug_data,
            delete_paper_ai_reading_session,
            get_ai_run,
            get_latest_paper_portrait_run,
            list_ai_runs,
            cancel_ai_run,
            list_ai_candidates,
            revise_ai_candidate,
            confirm_ai_overview_fields,
            undo_ai_overview_confirmation,
            confirm_ai_paper_review,
            undo_ai_paper_review_confirmation,
            decide_ai_candidate,
            decide_ai_suggestion_item,
            decide_ai_paper_concept_item,
            record_ai_citation_assessment,
            record_ai_evidence_inspection,
            record_ai_attention_feedback,
            create_ai_candidate_summary_version,
            select_ai_candidate_summary_version,
            discard_other_ai_candidate_summary_versions,
            create_project,
            list_note_pages,
            list_note_favorites,
            reorder_note_favorites,
            commit_note_navigation,
            create_note_page,
            create_literature_note,
            list_literature_notes,
            update_note_page,
            move_note_page,
            duplicate_note_page,
            set_note_page_status,
            apply_note_metadata_batch,
            replay_note_metadata_batch,
            list_note_page_metadata,
            permanently_delete_note_page,
            load_note_document,
            load_note_index_documents,
            query_note_page_catalog,


            notes::agent_context::agent_preview_note_context,
            ai::paper_context::preview::agent_preview_paper_context,
            ai::paper_context::prepare::agent_prepare_paper_context,
            agent::reading_submission::agent_prepare_paper_submission,
            agent::reading_submission::agent_prepare_paper_submission_proven,
            agent::reading_submission::prepare_status::agent_read_paper_submission_prepare_status,
            agent::reading_start::status::agent_read_paper_start_status,
            agent::reading_submission::agent_restore_paper_submission,
            agent::paper_recovery::agent_list_unstarted_paper_submissions,
            agent::reading_sources::agent_read_paper_turn_sources,
            agent::reading_start::agent_start_paper_submission,
            agent::reading_child_commands::agent_spawn_paper_child,
            agent::note_queue_preview::agent_prepare_note_queue_preview,
            agent::note_queue_preview::agent_read_pending_note_queue_preview,
            agent::note_queue_preview::agent_cancel_note_queue_preview,
            agent::note_queue_start::agent_start_note_queued_submission,
            agent::note_queue_recovery::agent_list_unstarted_note_queue_admissions,
            agent::note_steering::agent_steer_note_turn,
            agent::note_steering::agent_read_note_steering_recovery,
            agent::note_queue::agent_read_note_input_queue,
            agent::note_queue::agent_read_note_input_queue_edit_status,
            agent::note_queue::agent_edit_note_input_queue,
            agent::note_commands::agent_prepare_note_submission,
            agent::note_commands::agent_restore_note_submission,
            agent::note_commands::agent_list_unstarted_note_submissions,
            agent::note_review::agent_review_note_rewrite,
            agent::note_rejection::agent_reject_note_rewrite,
            agent::note_rejection::agent_note_rejection_status,
            agent::note_application::agent_accept_note_rewrite,
            agent::note_application::agent_note_application_status,
            agent::note_undo::agent_undo_note_rewrite,
            agent::note_undo::agent_note_undo_status,
            notes::reading_agent_context::agent_preview_reading_note_context,
            agent::reading_note_submission::agent_prepare_reading_note_submission,
            agent::reading_note_submission::agent_restore_reading_note_submission,
            agent::reading_note_start::agent_start_reading_note_submission,
            agent::reading_note_recovery::agent_list_unstarted_reading_note_submissions,
            agent::note_start::agent_start_note_submission,
            agent::compaction_commands::agent_start_compaction,
            agent::compaction_commands::preparation::agent_prepare_writing_compaction,
            agent::compaction_commands::preparation::agent_read_writing_compaction_prepare_status,
            agent::compaction_commands::preparation::agent_restore_writing_compaction,
            agent::context_usage_commands::agent_read_context_usage,
            agent::note_child_commands::agent_spawn_note_child,
            agent::note_child_commands::agent_spawn_reading_note_child,
            save_note_document,
            resolve_note_conflict,
            load_note_revision,
            list_note_revisions,
            set_note_revision_pinned,
            restore_note_revision,
            list_reading_notes,
            get_reading_note,
            create_reading_note,
            update_reading_note,
            set_reading_note_status,
            permanently_delete_reading_note,
            upsert_note_link,
            remove_note_link,
            list_note_links,
            list_note_source_labels,
            list_note_backlinks,
            search_notes,
            rebuild_note_index,
            cancel_note_index_rebuild,
            get_note_index_state,
            import_note_asset,
            cancel_note_asset_import,
            read_note_asset,
            export_notes,
            cancel_note_export,
            list_note_tags,
            ensure_note_tag,


















            preview_concept_merge,
            merge_concepts,
            split_concepts,
            list_concept_identity_events,
            apply_runtime_concept_identity,
            capture_concept_usage,
            save_runtime_definition_claim,
            list_runtime_definition_claims,
            list_runtime_research_object_revisions,
            save_definition_claim,
            list_definition_claims,
            search_relation_nodes,
            search_runtime_relation_nodes,
            save_relation_assertion,
            save_runtime_relation_assertion,
            list_relation_assertions,
            list_runtime_relation_assertions,



            list_research_object_revisions,
            get_semantic_index_status,
            get_semantic_embedding_job,
            retry_semantic_representation,
            cancel_semantic_index_job,
            run_semantic_index_reconciliation,
            rebuild_semantic_index,
            preview_concept_retrieval,
            cancel_concept_retrieval_preview,
            get_concept_retrieval_trace,
            analyze_concept_evolution,
            analyze_runtime_concept_evolution,
            analyze_shared_foundations,
            analyze_runtime_shared_foundations,
            get_concept_algorithm_run,
            get_runtime_concept_algorithm_run,


























































































            confirm_app_quit,
        ])
        .build(context)
        .expect("failed to build Research Assistant")
        .run(handle_run_event);
}
