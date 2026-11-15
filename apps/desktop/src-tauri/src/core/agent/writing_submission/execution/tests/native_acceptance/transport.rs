//! Frozen model responses that exercise existing patch and approved shell tools.
use super::*;
use crate::{agent::shell_tools, storage::AppStore};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum FixtureCase {
    Standard,
    MixedFiles,
    EditorDelete,
    EditorBinary,
    EditorOversized,
    EditorMode,
}

impl FixtureCase {
    pub(super) fn special(value: &str) -> Option<Self> {
        match value {
            "mixed-files" => Some(Self::MixedFiles),
            "editor-delete" => Some(Self::EditorDelete),
            "editor-binary" => Some(Self::EditorBinary),
            "editor-oversized" => Some(Self::EditorOversized),
            "editor-mode" => Some(Self::EditorMode),
            _ => None,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::MixedFiles => "mixed-files",
            Self::EditorDelete => "editor-delete",
            Self::EditorBinary => "editor-binary",
            Self::EditorOversized => "editor-oversized",
            Self::EditorMode => "editor-mode",
        }
    }

    pub(super) fn change_count(self) -> usize {
        if self == Self::MixedFiles { 6 } else { 2 }
    }

    fn shell_command(self) -> Option<&'static str> {
        match self {
            Self::Standard => None,
            Self::MixedFiles => Some(
                "printf '\\377\\376' > synthetic.bin && printf '\\376\\377' > new.bin && dd if=/dev/zero of=large.txt bs=1048576 count=5",
            ),
            Self::EditorDelete => Some("rm main.tex"),
            Self::EditorBinary => Some("printf '\\377\\376' > main.tex"),
            Self::EditorOversized => Some("dd if=/dev/zero of=main.tex bs=1048576 count=5"),
            Self::EditorMode => Some("chmod 700 main.tex"),
        }
    }

    fn operations(self) -> serde_json::Value {
        let main = serde_json::json!({"path":"main.tex", "before":MAIN,
            "after":MAIN.replace("Original abstract.", "Agent revised abstract.")
                .replace("Original conclusion.", "Agent revised conclusion.")});
        let other = if self == Self::Standard {
            serde_json::json!({"path":"sections/introduction.tex",
                "before":"Original introduction.","after":"Agent revised introduction."})
        } else {
            serde_json::json!({"path":"aaa.tex","before":null,
                "after":"\\section{Synthetic addition}\nAdded by the approved native patch tool.\n"})
        };
        let mut operations = vec![main, other];
        if self == Self::MixedFiles {
            operations.push(serde_json::json!({"path":"sections/introduction.tex",
                "before":INTRODUCTION,"after":null}));
        }
        serde_json::json!({"operations":operations})
    }

    pub(super) fn seed_binary(self, source: &Path, store: &AppStore, manuscript: Uuid) {
        if self != Self::MixedFiles {
            return;
        }
        let bytes = [0xff, 0x00, 0xfe];
        std::fs::write(source.join("synthetic.bin"), bytes).unwrap();
        store.connection().unwrap().execute(
            "insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,'synthetic.bin','other',?2,?3,'2026-10-07T00:00:00Z')",
            params![manuscript.to_string(),bytes.len(),format!("{:x}",Sha256::digest(bytes))],
        ).unwrap();
    }

    pub(super) fn assert_original_binary(self, source: &Path) {
        if self == Self::MixedFiles {
            assert_eq!(
                std::fs::read(source.join("synthetic.bin")).unwrap(),
                [0xff, 0x00, 0xfe]
            );
        }
    }
}

pub(super) struct PatchTransport {
    pub(super) calls: AtomicUsize,
    step: AtomicUsize,
    case: FixtureCase,
}

impl PatchTransport {
    pub(super) fn new(case: FixtureCase) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            step: AtomicUsize::new(0),
            case,
        }
    }
}

impl ModelTransport for PatchTransport {
    fn capabilities(&self) -> ModelCapabilities {
        Transport(AtomicUsize::new(0)).capabilities()
    }

    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(call < 10, "bounded native fixture response sequence");
        Box::pin(async move {
            for message in &request.messages {
                if let ModelMessage::ToolResult {
                    is_error: true,
                    content,
                    ..
                } = message
                {
                    panic!(
                        "synthetic tool failure: {}",
                        content.chars().take(1000).collect::<String>()
                    );
                }
            }
            let step = self.step.load(Ordering::SeqCst);
            let (name, arguments, next) = match step {
                0 => ("research_apply_patch", self.case.operations(), 1),
                1 if self.case.shell_command().is_some() => (
                    shell_tools::EXEC,
                    serde_json::json!({"command":self.case.shell_command().unwrap(),"yield_ms":1000}),
                    2,
                ),
                2 => {
                    let result = request
                        .messages
                        .iter()
                        .rev()
                        .find_map(|message| match message {
                            ModelMessage::ToolResult { content, .. } => Some(content),
                            _ => None,
                        })
                        .expect("approved shell must return its actual result");
                    let result: serde_json::Value = serde_json::from_str(result).unwrap();
                    match result["state"].as_str() {
                        Some("running") => (
                            shell_tools::PROCESS,
                            serde_json::json!({"action":"poll","process_id":result["processId"],"yield_ms":1000}),
                            2,
                        ),
                        Some("finished") => {
                            assert_eq!(result["exitCode"], 0, "approved shell result: {result}");
                            (
                                "research_review_workspace_changes",
                                serde_json::json!({}),
                                3,
                            )
                        }
                        _ => panic!("unexpected actual shell state: {result}"),
                    }
                }
                1 => (
                    "research_review_workspace_changes",
                    serde_json::json!({}),
                    3,
                ),
                3 if self.case == FixtureCase::MixedFiles => {
                    let result = request
                        .messages
                        .iter()
                        .rev()
                        .find_map(|message| match message {
                            ModelMessage::ToolResult { content, .. } => Some(content),
                            _ => None,
                        })
                        .unwrap();
                    let result: serde_json::Value = serde_json::from_str(result).unwrap();
                    let change = result["changes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|change| change["path"] == "synthetic.bin")
                        .unwrap();
                    (
                        "research_read_workspace_change",
                        serde_json::json!({
                            "path":change["path"], "before":change["before"], "after":change["after"]
                        }),
                        4,
                    )
                }
                4 => {
                    let result = request
                        .messages
                        .iter()
                        .rev()
                        .find_map(|message| match message {
                            ModelMessage::ToolResult { content, .. } => Some(content),
                            _ => None,
                        })
                        .unwrap();
                    let result: serde_json::Value = serde_json::from_str(result).unwrap();
                    assert_eq!(result["before"]["kind"], "binary");
                    assert_eq!(result["after"]["kind"], "binary");
                    return self.finish(request);
                }
                3 => return self.finish(request),
                _ => panic!("unexpected fixture response step"),
            };
            assert!(request.tools.iter().any(|tool| tool.name == name));
            self.step.store(next, Ordering::SeqCst);
            let id = format!("native-acceptance-{call}");
            Ok(Box::new(Stream(
                vec![
                    ModelStreamEvent::Started {
                        response_id: id.clone(),
                        actual_model: request.model,
                    },
                    ModelStreamEvent::ToolCallStarted {
                        call_id: id.clone(),
                        name: name.into(),
                    },
                    ModelStreamEvent::ToolArgumentsDelta {
                        call_id: id.clone(),
                        delta: arguments.to_string(),
                    },
                    ModelStreamEvent::ToolCallCompleted { call_id: id },
                    ModelStreamEvent::Completed {
                        reason: FinishReason::ToolCalls,
                        end_turn: None,
                    },
                ]
                .into(),
            )) as Box<dyn ModelStream>)
        })
    }
}

impl PatchTransport {
    fn finish(&self, request: ModelRequest) -> Result<Box<dyn ModelStream>, ModelError> {
        assert!(request.messages.iter().any(|message| matches!(
            message, ModelMessage::ToolResult {content,..} if content.contains("main.tex")
        )));
        if self.case == FixtureCase::Standard {
            assert!(request.messages.iter().any(|message| matches!(
                message, ModelMessage::ToolResult {content,..}
                    if content.contains("sections/introduction.tex")
            )));
        }
        Ok(Box::new(Stream(
            vec![
                ModelStreamEvent::Started {
                    response_id: "native-acceptance-finish".into(),
                    actual_model: request.model,
                },
                ModelStreamEvent::TextDelta(
                    if self.case == FixtureCase::Standard {
                        "Prepared two-file changes in the isolated copy."
                    } else {
                        "Prepared synthetic file changes in the isolated copy."
                    }
                    .into(),
                ),
                ModelStreamEvent::Completed {
                    reason: FinishReason::Stop,
                    end_turn: None,
                },
            ]
            .into(),
        )))
    }
}

#[test]
fn special_case_selection_requires_an_exact_explicit_name() {
    for name in [
        "mixed-files",
        "editor-delete",
        "editor-binary",
        "editor-oversized",
        "editor-mode",
    ] {
        let case = FixtureCase::special(name).unwrap();
        assert_eq!(case.name(), name);
        assert!(case.shell_command().is_some());
    }
    for name in [
        "",
        "standard",
        "../mixed-files",
        "editor-binary ",
        "unknown",
    ] {
        assert_eq!(FixtureCase::special(name), None);
    }
}
