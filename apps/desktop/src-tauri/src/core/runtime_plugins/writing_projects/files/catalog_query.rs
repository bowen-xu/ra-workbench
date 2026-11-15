//! Native catalog rules over the original coherent, indexed Core read.
mod grants;
mod read;
mod resources;
mod transaction;
use super::super::failure;
use crate::{
    runtime_plugins::{
        RuntimePluginCatalog,
        native::{
            DecidedProject, NativeHostError, NativePreviewService, NativeProfile, NativeResult,
            package,
        },
    },
    storage::AppStore,
};
pub(crate) use read::capture;
use research_workbench_native_sdk::{
    note_catalog_query::PageCatalogQuery,
    writing_catalog_query::{CatalogPage, QueryDecision, QueryInput, QueryProblem},
    writing_project::{Decision, Input},
};
pub(crate) use resources::QuerySql;
use std::sync::Arc;

pub(crate) fn notes_problem(problem: QueryProblem) -> crate::notes::NotesError {
    match problem {
        QueryProblem::InvalidInput { cause } => crate::notes::NotesError::InvalidInput(cause),
        QueryProblem::IndexFailed { cause } => crate::notes::NotesError::IndexFailed(cause),
        QueryProblem::CatalogChanged { current_revision } => {
            crate::notes::NotesError::CatalogChanged { current_revision }
        }
    }
}
fn rejected(problem: QueryProblem, key: &str) -> Box<NativeHostError> {
    let (code, revision) = match &problem {
        QueryProblem::InvalidInput { .. } => ("invalid_input", None),
        QueryProblem::IndexFailed { .. } => ("index_failed", None),
        QueryProblem::CatalogChanged { current_revision } => {
            ("revision_conflict", Some(*current_revision))
        }
    };
    let mut error = failure(code, notes_problem(problem).to_string(), key);
    error.current_revision = revision;
    error
}
fn reply(output: Decision, key: &str) -> NativeResult<QueryDecision> {
    match output {
        Decision::CatalogQuery { decision } => Ok(*decision),
        _ => Err(failure(
            "plugin_host_response_mismatch",
            "写作笔记目录查询决策种类不匹配。",
            key,
        )),
    }
}
async fn decide(
    service: &NativePreviewService,
    key: &str,
    input: QueryInput,
) -> NativeResult<(QueryDecision, Origin)> {
    let DecidedProject {
        output,
        instance_id,
        request_id,
    } = service
        .project_decision(
            key.into(),
            Input::CatalogQuery {
                input: Box::new(input),
            },
        )
        .await?;
    let origin = Origin {
        instance_id,
        request_id,
    };
    reply(output.decision, key)
        .map(|decision| (decision, origin.clone()))
        .map_err(|error| origin.attach(error))
}
#[derive(Clone)]
struct Origin {
    instance_id: String,
    request_id: String,
}
impl Origin {
    fn attach(&self, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
        error.instance_id = Some(self.instance_id.clone());
        error.request_id = Some(self.request_id.clone());
        error
    }
}
#[tauri::command]
pub(crate) async fn query_runtime_writing_note_catalog(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    query: PageCatalogQuery,
) -> NativeResult<CatalogPage> {
    query_catalog(
        store.inner().clone(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key,
        query,
    )
    .await
}
pub(crate) async fn query_catalog(
    store: AppStore,
    service: Arc<NativePreviewService>,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    query: PageCatalogQuery,
) -> NativeResult<CatalogPage> {
    let selected = key.clone();
    let operation_id = uuid::Uuid::new_v4().to_string();
    tokio::spawn(async move {
        let _operation = service.project_operation(&selected).await?;
        let (decision, origin) = decide(
            &service,
            &selected,
            QueryInput::Prepare {
                query: query.clone(),
            },
        )
        .await?;
        let plan = match decision {
            QueryDecision::Prepared { plan }
                if plan.query == query
                    && (1..=200).contains(&plan.limit)
                    && plan.terms.len() <= 32
                    && matches!(plan.status.as_str(), "active" | "archived") =>
            {
                plan
            }
            QueryDecision::Rejected { problem } => {
                return Err(origin.attach(rejected(problem, &selected)));
            }
            _ => {
                return Err(origin.attach(failure(
                    "plugin_host_response_mismatch",
                    "写作笔记查询准备与原请求不匹配。",
                    &selected,
                )));
            }
        };
        let read_plan = plan.clone();
        let read_key = selected.clone();
        let captured = tokio::task::spawn_blocking(move || {
            let package = package::accept(&catalog, &read_key, NativeProfile::WritingProjects)?;
            let sql = QuerySql::load(&catalog, &package)
                .map_err(|error| failure(error.code, error.message, &read_key))?;
            transaction::capture_owned(&store, &sql, &read_plan, &read_key)
        })
        .await
        .map_err(|_| {
            origin.attach(failure(
                "index_failed",
                "writing_note_catalog_task_failed：目录查询任务未能结算。",
                &selected,
            ))
        })?
        .map_err(|error| origin.attach(error))?;
        let (decision, origin) = decide(
            &service,
            &selected,
            QueryInput::Project {
                plan,
                snapshot: Box::new(captured),
            },
        )
        .await?;
        match decision {
            QueryDecision::Page { page } => Ok(page),
            QueryDecision::Rejected { problem } => Err(origin.attach(rejected(problem, &selected))),
            _ => Err(origin.attach(failure(
                "plugin_host_response_mismatch",
                "写作笔记目录查询未返回页面或原领域拒绝。",
                &selected,
            ))),
        }
    })
    .await
    .map_err(|_| {
        failure(
            "index_failed",
            "writing_note_catalog_task_failed：目录查询任务未能结算。",
            &key,
        )
    })?
    .map_err(|mut error| {
        error.operation_id = Some(operation_id);
        error
    })
}
