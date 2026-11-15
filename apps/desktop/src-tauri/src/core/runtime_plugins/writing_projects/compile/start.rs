//! Original queued-run, manager and background-worker order on the selected package.
use super::{domain::Domain, execution, read, repository, resources::Sql};
use crate::{
    compile_activity::CompileManager,
    runtime_plugins::{
        RuntimePluginCatalog,
        native::{NativePreviewService, NativeProfile, NativeResult, package},
        writing_projects::{context::Binding, data_failure, resources::ProjectReadResources},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    writing_compile::{
        CompileOptions, CompileRun, Completion, Decision, Input, LatexToolchainInfo,
    },
    writing_project,
};
use std::sync::Arc;
use tauri::Manager;

fn create(
    store: &AppStore,
    domain: &Domain,
    sql: &Sql,
    manuscript_id: &str,
    engine: &str,
) -> NativeResult<(CompileRun, String)> {
    let package = package::accept(
        &domain.binding.catalog,
        &domain.binding.package_key,
        NativeProfile::WritingProjects,
    )?;
    let project =
        ProjectReadResources::load(&domain.binding.catalog, &package).map_err(|error| {
            crate::runtime_plugins::writing_projects::failure(
                error.code,
                error.message,
                &domain.binding.package_key,
            )
        })?;
    let id = manuscript_id.trim();
    let rows = {
        let db = store
            .connection()
            .map_err(|cause| domain.error(cause.into()))?;
        project.get(&db, id).map_err(data_failure)?
    };
    let decision = domain.decide(writing_project::Input::GetProject {
        manuscript_id: id.into(),
        rows: rows.clone(),
    })?;
    let manuscript = match decision {
        writing_project::Decision::Project { manuscript } if manuscript.id == id => manuscript,
        writing_project::Decision::Rejected { reason } => return Err(domain.error(reason.into())),
        _ => return Err(domain.mismatch()),
    };
    let run = {
        let db = store
            .connection()
            .map_err(|cause| domain.error(cause.into()))?;
        project.recheck(&db, id, &rows).map_err(data_failure)?;
        repository::create_run(sql, &db, &manuscript.id, engine)
            .map_err(|cause| domain.error(cause))?
    };
    Ok((run, manuscript.root_path))
}

struct CompileJob {
    run: CompileRun,
    root: String,
    options: CompileOptions,
    toolchain: LatexToolchainInfo,
}

fn execute(
    store: &AppStore,
    domain: &Domain,
    sql: &Sql,
    job: &CompileJob,
    cancellation: Arc<std::sync::atomic::AtomicBool>,
    operation: crate::runtime_plugins::native::ProjectOperation,
) -> NativeResult<()> {
    let CompileJob {
        run,
        root,
        options,
        toolchain,
    } = job;
    domain.regenerate(store.clone(), run.manuscript_id.clone(), operation)?;
    let prepared = execution::prepare(store, sql, &run.id, &run.manuscript_id, root, toolchain)
        .map_err(|cause| domain.error(cause))?;
    let Decision::EngineArgument { argument } = domain.call(Input::EngineArgument {
        engine: run.engine.clone(),
    })?
    else {
        return Err(domain.mismatch());
    };
    let Decision::Flags { flags } = domain.call(Input::Flags {
        options: options.clone(),
    })?
    else {
        return Err(domain.mismatch());
    };
    let launched = execution::launch(
        &prepared,
        options,
        toolchain,
        argument,
        &flags,
        cancellation,
    )
    .map_err(|cause| domain.error(cause))?;
    let Decision::Completion {
        status,
        diagnostics,
    } = domain.call(Input::Completion {
        state: launched.state(),
        successful: launched.successful(),
        raw_output: launched.raw_output.clone(),
    })?
    else {
        return Err(domain.mismatch());
    };
    execution::finish(
        store,
        sql,
        &prepared,
        &launched,
        &Completion {
            status,
            diagnostics,
        },
        toolchain,
    )
    .map_err(|cause| domain.error(cause))
}

pub(crate) async fn start(
    store: AppStore,
    binding: Binding,
    manager: CompileManager,
    manuscript_id: String,
    engine: String,
    options: Option<CompileOptions>,
) -> NativeResult<CompileRun> {
    read::task_with_operation(store, binding, move |store, domain, sql, operation| {
        if !matches!(
            domain.call(Input::ValidateEngine {
                engine: engine.clone()
            })?,
            Decision::Accepted
        ) {
            return Err(domain.mismatch());
        }
        let toolchain = read::detect(&store, &domain)?;
        if !matches!(
            domain.call(Input::ValidateToolchain {
                engine: engine.clone(),
                toolchain: toolchain.clone()
            })?,
            Decision::Accepted
        ) {
            return Err(domain.mismatch());
        }
        let (run, root) = create(&store, &domain, &sql, &manuscript_id, &engine)?;
        let cancellation = match manager.register(&run.id, &run.manuscript_id) {
            Ok(cancellation) => cancellation,
            Err(error) => {
                let db = store
                    .connection()
                    .map_err(|cause| domain.error(cause.into()))?;
                repository::fail_internal(&sql, &db, &run.id, &error.to_string())
                    .map_err(|cause| domain.error(cause))?;
                return Err(domain.error(error));
            }
        };
        let job = CompileJob {
            run: run.clone(),
            root,
            options: options.unwrap_or_default(),
            toolchain,
        };
        tokio::task::spawn_blocking(move || {
            let result = execute(&store, &domain, &sql, &job, cancellation, operation.clone());
            if let Err(error) = result
                && let Ok(db) = store.connection()
            {
                let _ = repository::fail_internal(&sql, &db, &job.run.id, &error.to_string());
            }
            manager.finish(&job.run.id);
            drop(operation);
        });
        Ok(run)
    })
    .await
}

#[tauri::command]
pub(crate) async fn start_runtime_manuscript_compile(
    store: tauri::State<'_, AppStore>,
    app: tauri::AppHandle,
    package_key: String,
    manuscript_id: String,
    engine: String,
    options: Option<CompileOptions>,
) -> NativeResult<CompileRun> {
    let binding = Binding {
        service: Arc::clone(app.state::<Arc<NativePreviewService>>().inner()),
        catalog: Arc::clone(app.state::<Arc<RuntimePluginCatalog>>().inner()),
        package_key,
    };
    let manager = app.state::<CompileManager>().inner().clone();
    start(
        store.inner().clone(),
        binding,
        manager,
        manuscript_id,
        engine,
        options,
    )
    .await
}
