//! Native skill tool adapter. The host registers a frozen catalog and binds every
//! source to an authorized root. Model arguments cannot add roots or claim an
//! explicit user invocation. File I/O runs on the blocking pool.
use super::{
    files::{SkillReadError, load_selected},
    resources::{ResourceRequest, load_resource},
};
use crate::agent::instructions::{InstructionReadError, InstructionRootIdentity};
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolNameResolution, ToolOutput},
    skills::{
        SkillCatalog, SkillDescriptor, SkillError, SkillIdentity, SubmissionSkill, render_catalog,
        render_submission_catalog,
    },
};
use serde::Deserialize;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

const READ: &str = "research_load_skill";
const RESOURCE: &str = "research_read_skill_resource";
type Authorize =
    dyn Fn(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError> + Send + Sync;

#[derive(Clone)]
pub struct RegisteredSkill {
    pub descriptor: SkillDescriptor,
    pub root: PathBuf,
    /// Set by the host from user input, never from a model tool argument.
    pub explicit: bool,
    /// Rechecked for every load; must bind the root identity to current trust.
    pub authorize: Arc<Authorize>,
}

type TurnSkillSelection = Arc<dyn Fn(&SkillIdentity) -> bool + Send + Sync>;

pub struct SkillTools {
    skills: Vec<RegisteredSkill>,
    max_body_bytes: usize,
    selected_in_turn: Option<TurnSkillSelection>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoadArgs {
    identity: SkillIdentity,
}
impl SkillTools {
    pub fn new(skills: Vec<RegisteredSkill>, max_body_bytes: usize) -> Result<Self, SkillError> {
        if max_body_bytes == 0 || max_body_bytes > 1024 * 1024 {
            return Err(SkillError::Budget);
        }
        let descriptors: Vec<_> = skills
            .iter()
            .map(|skill| skill.descriptor.clone())
            .collect();
        // Validate the complete registration, including duplicates and omitted entries.
        render_catalog(&descriptors, 2)?;
        if skills.iter().any(|skill| !skill.root.is_absolute()) {
            return Err(SkillError::Invalid);
        }
        Ok(Self {
            skills,
            max_body_bytes,
            selected_in_turn: None,
        })
    }
    pub(crate) fn with_turn_selections(
        mut self,
        store: crate::storage::AppStore,
        turn: uuid::Uuid,
    ) -> Self {
        self.selected_in_turn = Some(Arc::new(move |identity| {
            let db = store.shared_agent_connection();
            let Ok(db) = db.lock() else {
                return false;
            };
            db.query_row("select exists(select 1 from agent_steering_resources r join agent_steering_inputs i on i.input_id=r.input_id, json_each(r.input_json) j where i.turn_id=?1 and json_extract(j.value,'$.type')='skill' and json_extract(j.value,'$.identity.environment_id')=?2 and json_extract(j.value,'$.identity.source_id')=?3 and json_extract(j.value,'$.identity.revision')=?4)", rusqlite::params![turn.to_string(), identity.environment_id, identity.source_id, identity.revision], |row| row.get(0)).unwrap_or(false)
        }));
        self
    }
    pub fn catalog(&self, max_bytes: usize) -> Result<SkillCatalog, SkillError> {
        render_submission_catalog(
            &self
                .skills
                .iter()
                .map(|skill| SubmissionSkill {
                    descriptor: skill.descriptor.clone(),
                    explicit: skill.explicit,
                })
                .collect::<Vec<_>>(),
            max_bytes,
        )
    }
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        if self.skills.is_empty()
            || (self.selected_in_turn.is_none()
                && !self
                    .skills
                    .iter()
                    .any(|skill| skill.explicit || skill.descriptor.allow_implicit_invocation))
        {
            return vec![];
        }
        let identity_schema = json!({"type":"object","properties":{
            "environment_id":{"type":"string"},"source_id":{"type":"string"},"revision":{"type":"string"}},
            "required":["environment_id","source_id","revision"],"additionalProperties":false});
        vec![ToolDefinition {
            namespace: None,
            name: READ.into(),
            description: "Load the full instructions for a skill from the supplied catalog using its exact identity and revision. This grants no additional tool or resource permissions.".into(),
            parameters: json!({"type":"object","properties":{"identity":identity_schema},
                "required":["identity"],"additionalProperties":false}),
        }, ToolDefinition {
            namespace: None,
            name: RESOURCE.into(),
            description: "Read a UTF-8 reference or script as text within a catalog skill's package. Load SKILL.md first and follow its resource references. Paths are relative to its directory; no execution, external paths, or links. Returns the resource's own revision; use expected_revision to require that version on reread (null for first read). Resources are read on demand, not frozen at submission.".into(),
            parameters: json!({"type":"object","properties":{"identity":identity_schema,
                "path":{"type":"string"},"expected_revision":{"type":["string","null"]}},
                "required":["identity","path","expected_revision"],"additionalProperties":false}),
        }]
    }
    fn selected(
        &self,
        call: &ToolCall,
    ) -> Result<(RegisteredSkill, Option<ResourceRequest>), RunError> {
        let (identity, resource) = match call.name.as_str() {
            READ => {
                let args: LoadArgs = crate::agent::tool_arguments::decode_object(
                    &call.arguments,
                    "Provide the exact skill identity and declared resource fields with their required types.",
                )?;
                (args.identity, None)
            }
            RESOURCE => {
                let args: ResourceRequest = crate::agent::tool_arguments::decode_object(
                    &call.arguments,
                    "Provide the exact skill identity and declared resource fields with their required types.",
                )?;
                args.validate().map_err(|_| RunError::ToolRejected)?;
                (args.identity.clone(), Some(args))
            }
            _ => return Err(RunError::ToolRejected),
        };
        let selected = self
            .skills
            .iter()
            .find(|skill| {
                skill.descriptor.identity == identity
                    && (skill.explicit
                        || skill.descriptor.allow_implicit_invocation
                        || self
                            .selected_in_turn
                            .as_ref()
                            .is_some_and(|selected| selected(&identity)))
            })
            .ok_or(RunError::ToolRejected)?;
        let mut selected = selected.clone();
        selected.explicit |= self
            .selected_in_turn
            .as_ref()
            .is_some_and(|selected_in_turn| selected_in_turn(&identity));
        Ok((selected, resource))
    }
}
impl ToolExecutor for SkillTools {
    fn requires_result_receipt(&self, call: &ToolCall) -> bool {
        // Both registered routes only load text and finish before returning;
        // this executor has no result_committed owner work.
        !matches!(call.name.as_str(), READ | RESOURCE)
    }
    fn resolve_name(&self, name: &str) -> ToolNameResolution {
        if matches!(name, READ | RESOURCE) {
            ToolNameResolution::Registered
        } else {
            ToolNameResolution::NotFound
        }
    }
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        self.selected(call).map(|_| ())
    }
    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            let (selected, resource) = self.selected(&call)?;
            let selected = selected.clone();
            let limit = self.max_body_bytes;
            let result = tokio::task::spawn_blocking(move || {
                if let Some(request) = resource {
                    load_resource(
                        &selected.root,
                        &selected.descriptor,
                        selected.explicit,
                        &request,
                        limit,
                        |identity| (selected.authorize)(identity),
                    )
                    .map(|resource| json!({"resource":resource}))
                } else {
                    load_selected(
                        &selected.root,
                        &selected.descriptor,
                        selected.explicit,
                        limit,
                        |identity| (selected.authorize)(identity),
                    )
                    .map(|loaded| json!({"skill":{"identity":loaded.identity,"name":selected.descriptor.name,"body":loaded.body,"executionDirectory":super::packages::relative(&loaded.identity)}}))
                }
            })
            .await
            .map_err(|_| RunError::ToolRejected)?;
            let (content, is_error) = match result {
                Ok(loaded) => (loaded.to_string(), false),
                Err(error) => {
                    let code = match error {
                        SkillReadError::NotFound => "not_found",
                        SkillReadError::Selection(SkillError::Stale) => "stale",
                        SkillReadError::Read(InstructionReadError::Unauthorized) => "unauthorized",
                        SkillReadError::Read(InstructionReadError::Budget)
                        | SkillReadError::Selection(SkillError::Budget) => "budget_exceeded",
                        _ => "unavailable",
                    };
                    (json!({"error":code,"message":"Skill or resource was not loaded. Do not assume its contents are available."}).to_string(), true)
                }
            };
            Ok(ToolOutput { content, is_error })
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::files::read_descriptor;
    use super::*;
    use std::{
        fs,
        path::Path,
        sync::atomic::{AtomicBool, Ordering},
    };
    #[tokio::test]
    async fn registered_tool_loads_and_reports_changed_or_revoked_sources() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir(root.join("review")).unwrap();
        let source = root.join("review/SKILL.md");
        fs::write(&source, "---\ndescription: Review\n---\nOriginal").unwrap();
        let descriptor =
            read_descriptor(&root, Path::new("review/SKILL.md"), "project", |_| Ok(())).unwrap();
        fs::write(root.join("review/reference.md"), "Resource text").unwrap();
        let mut explicit_only = descriptor.clone();
        explicit_only.allow_implicit_invocation = false;
        let disabled = SkillTools::new(
            vec![RegisteredSkill {
                descriptor: explicit_only,
                root: root.clone(),
                explicit: false,
                authorize: Arc::new(|_| panic!("disabled skill must not read files")),
            }],
            4096,
        )
        .unwrap();
        assert!(disabled.definitions().is_empty());
        assert_eq!(disabled.catalog(4096).unwrap().model_text, "[]");
        assert!(
            disabled
                .validate(&ToolCall {
                    namespace: None,
                    raw_arguments_json: None,
                    call_id: "disabled".into(),
                    name: READ.into(),
                    arguments: json!({"identity":descriptor.identity}),
                })
                .is_err()
        );
        let allowed = Arc::new(AtomicBool::new(true));
        let gate = allowed.clone();
        let tools = SkillTools::new(
            vec![RegisteredSkill {
                descriptor: descriptor.clone(),
                root,
                explicit: false,
                authorize: Arc::new(move |_| {
                    if gate.load(Ordering::SeqCst) {
                        Ok(())
                    } else {
                        Err(InstructionReadError::Unauthorized)
                    }
                }),
            }],
            4096,
        )
        .unwrap();
        let call = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "load".into(),
            name: READ.into(),
            arguments: json!({"identity":descriptor.identity}),
        };
        assert!(!tools.requires_result_receipt(&call));
        assert_eq!(tools.definitions().len(), 2);
        let output = tools.authorize_and_execute(call.clone()).await.unwrap();
        assert!(!output.is_error);
        let loaded: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(loaded["skill"]["name"], descriptor.name);
        assert!(output.content.contains("Original"));
        assert!(!tools.catalog(4096).unwrap().model_text.contains("Original"));
        let resource_call = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "resource".into(),
            name: RESOURCE.into(),
            arguments: json!({"identity":descriptor.identity,"path":"reference.md","expected_revision":null}),
        };
        assert!(!tools.requires_result_receipt(&resource_call));
        let mut unknown_call = call.clone();
        unknown_call.name = "unknown".into();
        assert!(tools.requires_result_receipt(&unknown_call));
        let resource = tools
            .authorize_and_execute(resource_call.clone())
            .await
            .unwrap();
        assert!(!resource.is_error);
        assert!(resource.content.contains("Resource text"));
        assert!(resource.content.contains("revision"));
        let mut forged_resource = resource_call.clone();
        forged_resource.arguments["path"] = json!("../outside.md");
        assert!(tools.validate(&forged_resource).is_err());
        forged_resource.arguments =
            json!({"identity":descriptor.identity,"path":"reference.md","root":"/tmp"});
        assert!(tools.validate(&forged_resource).is_err());
        fs::write(&source, "---\ndescription: Review\n---\nChanged").unwrap();
        let output = tools.authorize_and_execute(call.clone()).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("stale"));
        assert!(
            tools
                .authorize_and_execute(resource_call.clone())
                .await
                .unwrap()
                .content
                .contains("stale")
        );
        allowed.store(false, Ordering::SeqCst);
        assert!(
            tools
                .authorize_and_execute(resource_call)
                .await
                .unwrap()
                .content
                .contains("unauthorized")
        );
        assert!(
            tools
                .authorize_and_execute(call.clone())
                .await
                .unwrap()
                .content
                .contains("unauthorized")
        );
        let mut forged = call;
        forged.arguments["explicit"] = json!(true);
        assert!(tools.validate(&forged).is_err());
        forged.arguments = json!({"identity":{"environment_id":"other", "source_id":"review/SKILL.md", "revision":"unknown"}});
        assert!(tools.validate(&forged).is_err());
    }
}
