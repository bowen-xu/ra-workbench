//! Recheck the original snapshot selector and history around the reuse decision.
use super::*;
use crate::runtime_plugins::{
    data::load_text,
    literature_analysis::{capture, resources::HistoryResources},
};
use research_workbench_native_sdk::analysis_history::HistoryRequestV1;
use rusqlite::{Connection, OptionalExtension, params};

impl LiteratureTraceService {
    pub(super) async fn validate_analysis_reuse(
        self: &Arc<Self>,
        store: AppStore,
        key: &str,
        draft: &PaperAnalysisStartDraft,
        snapshot: &str,
        template_json: Option<String>,
    ) -> Result<(), AiError> {
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        let paper = draft.paper_id.clone();
        let snapshot = snapshot.to_owned();
        let source = store.clone();
        let request_paper = paper.clone();
        let request_snapshot = snapshot.clone();
        let (sql, history, id) = tokio::task::spawn_blocking(move || {
            let package = package::accept_analysis_workflow(&catalog, &selected)
                .map_err(template::technical)?;
            package::accept_analysis_history(&catalog, &selected).map_err(template::technical)?;
            let load_error = |error: crate::runtime_plugins::RuntimePluginError| {
                template::technical(
                    LiteratureTraceService::error(error.code, error.message).package(&selected),
                )
            };
            let sql = load_text(
                &catalog,
                &package,
                "literature_library",
                "data/analysis-workflow-latest-context.v1.sql",
                "f3791360fb410cbfe3e7ca80eae2d712de01c881dec9687a2636ba3606df03f4",
            )
            .map_err(load_error)?;
            let history = HistoryResources::load(&catalog, &package).map_err(load_error)?;
            let db = source.connection()?;
            let id = select(&db, &sql, &request_paper, &request_snapshot)?;
            Ok::<_, AiError>((sql, history, id))
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)??;
        let observed = if let Some(id) = &id {
            Some(
                self.read_analysis_history_observed_ai(
                    store.clone(),
                    key.into(),
                    HistoryRequestV1::Get {
                        workflow_id: id.clone(),
                    },
                )
                .await?,
            )
        } else {
            None
        };
        let (decision, origin) = self
            .analysis_workflow_decision(
                key,
                WorkflowInputV1::ValidateReuse {
                    regenerate: draft.regenerate,
                    template_json,
                    existing: observed
                        .as_ref()
                        .and_then(|read| read.workflow.clone())
                        .map(Box::new),
                },
            )
            .await
            .map_err(template::technical)?;
        let result = match decision {
            WorkflowDecisionV1::Validated { rejection: None } => Ok(()),
            WorkflowDecisionV1::Validated {
                rejection: Some(error),
            } => Err(template::rejected(error, &origin)),
            _ => Err(template::identified(
                "plugin_host_response_mismatch",
                "分析重复生成校验种类不匹配。",
                &origin,
            )),
        };
        tokio::task::spawn_blocking(move || {
            let db = store.connection()?;
            let current = select(&db, &sql, &paper, &snapshot)?;
            if current != id {
                return Err(changed(&origin));
            }
            if let Some(observed) = observed {
                let current =
                    capture::capture(&db, &observed.input.request, &history.statements())?;
                if current != observed.input.workflow {
                    return Err(changed(&origin));
                }
            }
            result
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?
    }
}
fn select(
    db: &Connection,
    sql: &str,
    paper: &str,
    snapshot: &str,
) -> Result<Option<String>, AiError> {
    db.query_row(sql, params![paper, snapshot], |row| row.get(0))
        .optional()
        .map_err(Into::into)
}
fn changed(origin: &DecisionOrigin) -> AiError {
    template::identified(
        "plugin_host_source_changed",
        "论文分析重复生成检查的来源已变化，请重新载入。",
        origin,
    )
}
