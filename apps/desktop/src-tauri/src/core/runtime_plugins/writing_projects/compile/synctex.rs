//! Formal SyncTeX endpoints reuse the original scoped IO and selected writing rules.
use super::{domain::Domain, query, read, repository, resources::Sql, synctex_io};
use crate::agent::compile_synctex::AgentCompileSynctexQuery;
use crate::{
    runtime_plugins::{
        RuntimePluginCatalog,
        native::{NativePreviewService, NativeResult},
        writing_projects::context::Binding,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::writing_compile::{
    Decision, Input, SynctexPdfLocation, SynctexSourceLocation,
};
use std::sync::Arc;

fn paths(
    store: &AppStore,
    domain: &Domain,
    sql: &Sql,
    run_id: &str,
) -> NativeResult<(repository::SynctexRunContext, synctex_io::RunPaths)> {
    let context = {
        let db = store
            .connection()
            .map_err(|cause| domain.error(cause.into()))?;
        repository::synctex_run_context(sql, &db, run_id).map_err(|cause| domain.error(cause))?
    };
    synctex_io::ensure_current_input(
        store.manuscript_directory(),
        &context.manuscript_id,
        &context.input_manifest_hash,
    )
    .map_err(|cause| domain.error(cause))?;
    let paths = synctex_io::run_paths(
        store.writing_cache_directory(),
        run_id,
        &context.manuscript_id,
        &context.root_path,
    )
    .map_err(|cause| domain.error(cause))?;
    Ok((context, paths))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn forward(
    store: &AppStore,
    domain: &Domain,
    sql: &Sql,
    run_id: &str,
    relative_path: &str,
    line: u32,
    column: u32,
) -> NativeResult<Vec<SynctexPdfLocation>> {
    if !matches!(
        domain.call(Input::ValidateForward {
            run_id: Some(run_id.into()),
            relative_path: relative_path.into(),
            line,
        })?,
        Decision::Accepted
    ) {
        return Err(domain.mismatch());
    }
    // Preserve the original path codec before the context read.
    crate::scoped_text_files::legacy::validate_relative_path(relative_path)
        .map_err(|cause| domain.error(cause.into()))?;
    let (context, paths) = paths(store, domain, sql, run_id)?;
    query::forward(
        domain,
        &AgentCompileSynctexQuery {
            runtime_home: &store.writing_cache_directory().join("toolchain-home"),
            toolchain_json: &context.toolchain_json,
            snapshot: &paths.snapshot,
            output_directory: &paths.output_directory,
            pdf: &paths.pdf,
        },
        relative_path,
        line,
        column,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn inverse(
    store: &AppStore,
    domain: &Domain,
    sql: &Sql,
    run_id: &str,
    page: u32,
    x: f64,
    y: f64,
) -> NativeResult<Vec<SynctexSourceLocation>> {
    if !matches!(
        domain.call(Input::ValidateInverse {
            run_id: Some(run_id.into()),
            page,
            x_bits: x.to_bits(),
            y_bits: y.to_bits(),
        })?,
        Decision::Accepted
    ) {
        return Err(domain.mismatch());
    }
    let (context, paths) = paths(store, domain, sql, run_id)?;
    query::inverse(
        domain,
        &AgentCompileSynctexQuery {
            runtime_home: &store.writing_cache_directory().join("toolchain-home"),
            toolchain_json: &context.toolchain_json,
            snapshot: &paths.snapshot,
            output_directory: &paths.output_directory,
            pdf: &paths.pdf,
        },
        page,
        x,
        y,
    )
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn forward_runtime_manuscript_synctex(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    run_id: String,
    relative_path: String,
    line: u32,
    column: u32,
) -> NativeResult<Vec<SynctexPdfLocation>> {
    read::task(
        store.inner().clone(),
        Binding {
            service: service.inner().clone(),
            catalog: catalog.inner().clone(),
            package_key,
        },
        move |store, domain, sql| {
            forward(
                &store,
                &domain,
                &sql,
                run_id.trim(),
                relative_path.trim(),
                line,
                column,
            )
        },
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn inverse_runtime_manuscript_synctex(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    run_id: String,
    page: u32,
    x: f64,
    y: f64,
) -> NativeResult<Vec<SynctexSourceLocation>> {
    read::task(
        store.inner().clone(),
        Binding {
            service: service.inner().clone(),
            catalog: catalog.inner().clone(),
            package_key,
        },
        move |store, domain, sql| inverse(&store, &domain, &sql, run_id.trim(), page, x, y),
    )
    .await
}
