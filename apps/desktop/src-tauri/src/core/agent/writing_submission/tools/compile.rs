//! Read-only compiler feedback for a manuscript-bound Agent turn.
//!
//! Compilation itself remains owned by the writing compiler coordinator. This
//! tool only exposes the latest persisted run and a bounded diagnostic/log
//! view, after re-checking the conversation-to-manuscript binding.
use super::*;
use crate::{agent::writing_compile_contracts::CompileRun, storage::AppStore};
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolOutput},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

pub(super) const READ_DIAGNOSTICS: &str = "research_read_compile_diagnostics";
pub(super) const FORWARD_SYNCTEX: &str = "research_forward_synctex";
const MAX_RAW_OUTPUT: usize = 12_000;

#[path = "compile/runtime.rs"]
mod runtime;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    #[serde(default)]
    include_raw_output: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ForwardRequest {
    relative_path: String,
    line: u32,
    #[serde(default)]
    column: u32,
}

pub(super) struct CompileTools {
    store: AppStore,
    conversation_id: Uuid,
    manuscript_id: Uuid,
}

impl CompileTools {
    pub(super) fn new(
        store: AppStore,
        record: &SubmissionRecord,
    ) -> Result<Self, WritingSubmissionError> {
        Ok(Self {
            store,
            conversation_id: record.conversation_id,
            manuscript_id: record
                .input
                .context
                .iter()
                .find_map(|source| {
                    source
                        .anchor
                        .as_deref()
                        .and_then(|anchor| serde_json::from_str::<serde_json::Value>(anchor).ok())
                        .and_then(|anchor| {
                            anchor
                                .get("manuscriptId")?
                                .as_str()
                                .and_then(|id| Uuid::parse_str(id).ok())
                        })
                })
                .ok_or(SubmissionError::Corrupt)?,
        })
    }

    pub(super) fn definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            namespace: None,
            name: READ_DIAGNOSTICS.into(),
            description: "Read the latest persisted LaTeX compile status, diagnostics, and artifact summary for the bound manuscript. This is read-only and never starts a compile. Raw compiler output is omitted unless requested and is capped at 12000 characters.".into(),
            parameters: json!({
                "type": "object",
                "properties": {"includeRawOutput": {"type": "boolean"}},
                "additionalProperties": false
            }),
        }, ToolDefinition {
            namespace: None,
            name: FORWARD_SYNCTEX.into(),
            description: "Map a source file and line in the latest successful, current manuscript compile to PDF page coordinates using SyncTeX. This is read-only; stale source, missing SyncTeX, or no matching location is returned as a tool error.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "relativePath": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "line": {"type": "integer", "minimum": 1, "maximum": 10000000},
                    "column": {"type": "integer", "minimum": 0, "maximum": 10000000}
                },
                "required": ["relativePath", "line"],
                "additionalProperties": false
            })
        }]
    }

    fn parse(call: &ToolCall) -> Result<ParsedRequest, RunError> {
        if call.call_id.is_empty() || call.call_id.len() > 512 {
            return Err(RunError::ToolRejected);
        }
        match call.name.as_str() {
            READ_DIAGNOSTICS => crate::agent::tool_arguments::decode_object(
                &call.arguments,
                "Provide only the declared diagnostics fields with their required types.",
            )
            .map(ParsedRequest::Diagnostics),
            FORWARD_SYNCTEX => {
                let request: ForwardRequest = crate::agent::tool_arguments::decode_object(
                    &call.arguments,
                    "Provide only the declared diagnostics or SyncTeX fields with their required types.",
                )?;
                if request.relative_path.contains('\0') || request.line == 0 {
                    return Err(RunError::ToolRejected);
                }
                Ok(ParsedRequest::Forward(request))
            }
            _ => Err(RunError::ToolRejected),
        }
    }

    async fn read(&self, request: ParsedRequest) -> Result<ToolOutput, RunError> {
        let store = self.store.clone();
        let conversation_id = self.conversation_id;
        let manuscript_id = self.manuscript_id;
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let connection = store.connection().map_err(|_| RunError::EventSink)?;
            super::super::check_binding(&connection, conversation_id, manuscript_id)
                .map_err(|_| RunError::ToolRejected)?;
            let Some(source) = store.writing_compile_feedback_source() else {
                return runtime::read(&store, connection, manuscript_id, request, runtime);
            };
            match request {
                ParsedRequest::Diagnostics(request) => {
                    let run = (source.latest)(&connection, manuscript_id)
                        .map_err(|_| RunError::EventSink)?;
                    Ok(render(run, request.include_raw_output))
                }
                ParsedRequest::Forward(request) => {
                    let run = (source.latest_pdf)(&connection, manuscript_id)
                        .map_err(|_| RunError::EventSink)?
                        .ok_or(RunError::ToolRejected)?;
                    drop(connection);
                    let result = (|| {
                        let query = (source.validate_forward)(
                            &run.id,
                            &request.relative_path,
                            request.line,
                            request.column,
                        )?;
                        let context = {
                            let connection =
                                store.connection().map_err(|error| error.to_string())?;
                            (source.forward_context)(&connection, &run.id)?
                        };
                        (source.forward)(
                            store.manuscript_directory(),
                            store.writing_cache_directory(),
                            &query,
                            &context,
                        )
                    })();
                    match result {
                        Ok(locations) => Ok(ToolOutput {
                            content: serde_json::to_string(&locations)
                                .map_err(|_| RunError::EventSink)?,
                            is_error: false,
                        }),
                        Err(error) => Ok(ToolOutput {
                            content: format!("SyncTeX unavailable: {error}"),
                            is_error: true,
                        }),
                    }
                }
            }
        })
        .await
        .map_err(|_| RunError::EventSink)?
    }
}

impl ToolExecutor for CompileTools {
    fn requires_result_receipt(&self, call: &ToolCall) -> bool {
        // Both operations finish before returning. Their result callback only
        // repeats validation and owns no resource to acknowledge.
        !matches!(call.name.as_str(), READ_DIAGNOSTICS | FORWARD_SYNCTEX)
    }
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        Self::parse(call).map(|_| ())
    }

    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            let request = Self::parse(&call)?;
            self.read(request).await
        })
    }

    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        Box::pin(async move { Self::parse(&call).map(|_| ()) })
    }
}

enum ParsedRequest {
    Diagnostics(Request),
    Forward(ForwardRequest),
}

fn render(run: Option<CompileRun>, include_raw_output: bool) -> ToolOutput {
    let content = match run {
        None => json!({"status": "no_compile_run"}),
        Some(run) => {
            let mut value = json!({
                "status": run.status,
                "runId": run.id,
                "manuscriptId": run.manuscript_id,
                "engine": run.engine,
                "diagnostics": run.diagnostics,
                "artifacts": run.artifacts,
                "exitCode": run.exit_code,
                "startedAt": run.started_at,
                "finishedAt": run.finished_at,
            });
            if include_raw_output {
                value["rawOutput"] = json!(tail(&run.raw_output, MAX_RAW_OUTPUT));
            }
            value
        }
    };
    ToolOutput {
        content: content.to_string(),
        is_error: false,
    }
}

fn tail(value: &str, maximum_characters: usize) -> String {
    let count = value.chars().count();
    if count <= maximum_characters {
        return value.to_owned();
    }
    let start = value
        .char_indices()
        .nth(count - maximum_characters)
        .map_or(value.len(), |(index, _)| index);
    format!("…{}", &value[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_receipt_policy_is_limited_to_compiler_feedback() {
        let f = super::super::super::tests::fixture(true);
        let service = WritingSubmissionService::from_app_store(&f.store);
        let input = super::super::super::tests::request(&f, Uuid::new_v4());
        let record = service.prepare(&f.models, input).await.unwrap().record;
        let tools = CompileTools::new(f.store.clone(), &record).unwrap();
        let mut call = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "read".into(),
            name: READ_DIAGNOSTICS.into(),
            arguments: json!({}),
        };
        assert!(!tools.requires_result_receipt(&call));
        assert!(
            !tools
                .authorize_and_execute(call.clone())
                .await
                .unwrap()
                .is_error
        );
        call.name = FORWARD_SYNCTEX.into();
        call.arguments = json!({"relativePath":"main.tex", "line":1});
        assert!(tools.validate(&call).is_ok());
        assert!(!tools.requires_result_receipt(&call));
        call.name = "unknown".into();
        assert!(tools.requires_result_receipt(&call));
    }

    #[test]
    fn accepts_only_the_bounded_read_request() {
        let call = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "call-1".into(),
            name: READ_DIAGNOSTICS.into(),
            arguments: json!({"includeRawOutput": true}),
        };
        assert!(CompileTools::parse(&call).is_ok());
        let mut invalid = call.clone();
        invalid.arguments = json!({"includeRawOutput": true, "manuscriptId": Uuid::new_v4()});
        assert!(CompileTools::parse(&invalid).is_err());
    }

    #[test]
    fn caps_raw_output_from_the_tail() {
        let run = CompileRun {
            id: "run".into(),
            manuscript_id: "manuscript".into(),
            engine: "pdflatex".into(),
            status: "failed".into(),
            diagnostics: vec![],
            artifacts: vec![],
            raw_output: format!("{}中文日志", "x".repeat(MAX_RAW_OUTPUT)),
            exit_code: Some(1),
            started_at: None,
            finished_at: None,
        };
        let output = render(Some(run), true);
        let value: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(
            value["rawOutput"].as_str().unwrap().chars().count(),
            MAX_RAW_OUTPUT + 1
        );
        assert!(value["rawOutput"].as_str().unwrap().starts_with('…'));
        assert!(value["rawOutput"].as_str().unwrap().ends_with("中文日志"));
    }

    #[test]
    fn validates_forward_synctex_identity_and_line() {
        let call = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "call-2".into(),
            name: FORWARD_SYNCTEX.into(),
            arguments: json!({"relativePath": "sections/body.tex", "line": 12}),
        };
        assert!(matches!(
            CompileTools::parse(&call),
            Ok(ParsedRequest::Forward(_))
        ));
        let mut invalid = call;
        invalid.arguments = json!({"relativePath": "sections/body.tex", "line": 0});
        assert!(CompileTools::parse(&invalid).is_err());
    }

    #[tokio::test]
    async fn binding_rejection_precedes_history_and_history_sql_errors_keep_event_sink() {
        let f = super::super::super::tests::fixture(false);
        let tools = CompileTools {
            store: f.store.clone(),
            conversation_id: f.conversation,
            manuscript_id: f.manuscript,
        };
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop table manuscript_compile_runs")
            .unwrap();
        let diagnostics = || {
            ParsedRequest::Diagnostics(Request {
                include_raw_output: false,
            })
        };
        let forward = || {
            ParsedRequest::Forward(ForwardRequest {
                relative_path: "main.tex".into(),
                line: 1,
                column: 0,
            })
        };
        // A domain call would fail with EventSink; binding must reject it first.
        assert!(matches!(
            tools.read(diagnostics()).await,
            Err(RunError::ToolRejected)
        ));
        assert!(matches!(
            tools.read(forward()).await,
            Err(RunError::ToolRejected)
        ));
        f.store.connection().unwrap().execute(
            "insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'manuscript',?2)",
            rusqlite::params![f.conversation.to_string(), f.manuscript.to_string()],
        ).unwrap();
        assert!(matches!(
            tools.read(diagnostics()).await,
            Err(RunError::EventSink)
        ));
        assert!(matches!(
            tools.read(forward()).await,
            Err(RunError::EventSink)
        ));
    }

    #[tokio::test]
    async fn latest_feedback_keeps_pdf_selection_and_formal_error_categories() {
        let f = super::super::super::tests::fixture(true);
        let tools = CompileTools {
            store: f.store.clone(),
            conversation_id: f.conversation,
            manuscript_id: f.manuscript,
        };
        let forward = |path: &str| {
            ParsedRequest::Forward(ForwardRequest {
                relative_path: path.into(),
                line: 7,
                column: u32::MAX,
            })
        };
        assert!(matches!(
            tools.read(forward("main.tex")).await,
            Err(RunError::ToolRejected)
        ));
        let pdf_run = "AAAAAAAA-1111-4111-8111-111111111111";
        let latest_run = Uuid::new_v4().to_string();
        let source = f
            .store
            .writing_compile_feedback_source()
            .expect("writing fixture compile feedback");
        {
            let db = f.store.connection().unwrap();
            for (id, status) in [(pdf_run, "failed"), (latest_run.as_str(), "cancelled")] {
                db.execute(
                    "insert into manuscript_compile_runs(id,manuscript_id,engine,root_path,status,created_at) values(?1,?2,'pdflatex','main.tex',?3,'same-time')",
                    rusqlite::params![id,f.manuscript.to_string(),status],
                ).unwrap();
                db.execute(
                    "insert into manuscript_compile_artifacts(id,run_id,artifact_kind,cache_relative_path,byte_size) values(?1,?2,'pdf','synthetic.pdf',9)",
                    rusqlite::params![Uuid::new_v4().to_string(),id],
                ).unwrap();
            }
            assert_eq!(
                (source.latest)(&db, f.manuscript).unwrap().unwrap().id,
                latest_run
            );
            let selected = (source.latest_pdf)(&db, f.manuscript).unwrap().unwrap();
            assert_eq!(selected.id, pdf_run);
            assert_eq!(selected.status, "failed");
            assert_eq!(
                serde_json::to_value(&selected.artifacts).unwrap(),
                json!([{
                    "kind":"pdf","byteSize":9,"contentHash":null
                }])
            );
        }
        let diagnostics = tools
            .read(ParsedRequest::Diagnostics(Request {
                include_raw_output: false,
            }))
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&diagnostics.content).unwrap();
        assert_eq!(value["runId"], latest_run);
        assert_eq!(value["status"], "cancelled");

        // Path rejection precedes the missing formal SyncTeX context. Neither
        // error becomes a runner failure or a temporary Agent artifact lookup.
        let invalid_path = "../main.tex";
        let expected = (source.validate_forward)(pdf_run, invalid_path, 7, u32::MAX)
            .err()
            .unwrap();
        let rejected = tools.read(forward(invalid_path)).await.unwrap();
        assert!(rejected.is_error);
        assert_eq!(rejected.content, format!("SyncTeX unavailable: {expected}"));
        let expected = (source.forward_context)(&f.store.connection().unwrap(), pdf_run)
            .err()
            .unwrap();
        let unavailable = tools.read(forward("main.tex")).await.unwrap();
        assert!(unavailable.is_error);
        assert_eq!(
            unavailable.content,
            format!("SyncTeX unavailable: {expected}")
        );
    }
}
