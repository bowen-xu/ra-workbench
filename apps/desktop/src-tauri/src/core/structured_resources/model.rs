use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TEMPLATE_CATALOG_ID: &str = "8bda6f39-8371-4d69-90b8-9f1b1ca8ab01";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resource {
    pub id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper_id: Option<String>,
    pub source_uri: String,
    pub title: String,
    pub revision: i64,
    pub content: Option<Value>,
    pub review_baseline: Option<Value>,
    pub pending_field_ids: Vec<String>,
    pub reviewed_field_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_ref: Option<TemplateRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<Draft>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateRef {
    pub resource_id: String,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationIssue {
    pub path: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Draft {
    pub id: String,
    pub resource_id: String,
    pub base_revision: i64,
    pub revision: i64,
    pub content: String,
    pub content_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_conversation_id: Option<String>,
    pub errors: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteDraft {
    pub resource_id: String,
    pub expected_revision: i64,
    pub content: String,
    pub request_id: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub expected_draft_revision: Option<i64>,
    #[serde(default)]
    pub source_snapshot_id: Option<String>,
    #[serde(default)]
    pub submission_id: Option<String>,
    #[serde(default = "default_editor")]
    pub editor_kind: String,
}
fn default_editor() -> String {
    "human".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteResult {
    pub status: String,
    pub resource: Resource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<Draft>,
    pub errors: Vec<ValidationIssue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_origin: Option<ValidationOrigin>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationOrigin {
    pub request_id: String,
    pub package_key: String,
    pub instance_id: String,
    pub native_request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    pub request_id: String,
    pub resource_id: String,
    pub revision: i64,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextEdit {
    pub old_text: String,
    pub new_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchDraft {
    pub resource_id: String,
    pub draft_id: String,
    pub expected_revision: i64,
    pub expected_draft_revision: i64,
    pub request_id: String,
    pub edits: Vec<TextEdit>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub submission_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewDraft {
    pub resource_id: String,
    pub expected_revision: i64,
    pub request_id: String,
    pub action: String,
    #[serde(default)]
    pub field_ids: Option<Vec<String>>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryDraft {
    pub resource_id: String,
    pub expected_revision: i64,
    pub operation_revision: i64,
    pub direction: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdoptionDraft {
    pub paper_id: String,
    pub template_id: String,
    pub template_revision: i64,
    pub expected_revision: i64,
    pub confirmed: bool,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyImportDraft {
    pub request_id: String,
    #[serde(default)]
    pub catalog: Option<Value>,
    #[serde(default)]
    pub paper_id: Option<String>,
    #[serde(default)]
    pub presentation: Option<LegacyPresentation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyPresentation {
    pub fields: Vec<LegacyField>,
    pub hidden_unit_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyField {
    pub candidate_id: String,
    pub field_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadResult {
    pub resource: Resource,
    pub text: String,
    pub draft: Option<Draft>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartupImportDraft {
    #[serde(default)]
    pub catalog: Option<Value>,
    #[serde(default)]
    pub presentations: Vec<StartupPresentation>,
    #[serde(default)]
    pub contexts: Vec<StartupContext>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartupPresentation {
    pub paper_id: String,
    pub presentation: LegacyPresentation,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartupContext {
    pub conversation_id: uuid::Uuid,
    pub state: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartupImportResult {
    pub paper_ids: Vec<String>,
    pub imported_paper_count: usize,
    pub imported_template_count: usize,
    pub imported_context_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deferred_paper_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deferred_context_ids: Vec<uuid::Uuid>,
}
