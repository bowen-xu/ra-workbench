use super::*;
use crate::ai::{
    generation_contracts::ProviderImage,
    generation_support::{ProviderInput, ProviderMessage, ProviderMessageRole},
    structured_generation_support::{decorate_submission, prompt_resource},
};
use research_agent_core::submission::{ContextSourceKind, ContextText};
use research_workbench_native_sdk::analysis_workflow::prompt::{PaperTaskPrompt, PromptResourceV1};
use serde_json::{Value, json};

fn grant(id: &str, resource: Value, kind: Option<&str>) -> ContextText {
    ContextText {
        source_id: format!("grant:{id}"),
        kind: ContextSourceKind::Paper,
        resource_id: id.into(),
        revision: format!("frozen:{id}"),
        anchor: Some(
            json!({"content":"resourceGrant","resource":resource,
            "sourceUrl":format!("source://library/paper/{id}"),"resourceKind":kind,
            "contextAccess":"readWrite"})
            .to_string(),
        ),
        text: "Available; body not loaded".into(),
    }
}
fn grants(kind: &str) -> Vec<ContextText> {
    vec![
        grant(
            "paper",
            json!({"kind":"paper","resourceId":"paper","attachmentId":"pdf-1"}),
            None,
        ),
        grant(
            "target",
            json!({"kind":"structuredObject","resourceId":"target","access":"readWrite"}),
            Some(kind),
        ),
        grant(
            "template",
            json!({"kind":"structuredObject","resourceId":"template","access":"readOnly"}),
            Some("readingTemplate"),
        ),
    ]
}

// A fixed accepted policy response keeps Core admission and skill-binding tests local.
fn accepted_prompt() -> PaperTaskPrompt {
    let text = "将画像以 paper_portrait_v1 JSON 对象写入目标文件。配置 🧪".to_owned();
    PaperTaskPrompt {
        user_input: vec![UserInput::Text {
            text: text.clone(),
            text_elements: vec![],
        }],
        text,
    }
}

#[test]
fn outgoing_provider_user_message_is_the_admitted_task_not_the_legacy_envelope() {
    let grants = grants("structuredReading");
    let prompt = accepted_prompt();
    let text = prompt.text.clone();
    let submission = research_agent_core::submission::SubmissionInput {
        remote_compaction_v2: None,
        text,
        user_input: prompt.user_input.clone(),
        context: grants.clone(),
        model: research_agent_core::protocol::ModelIdentity {
            provider_id: "test".into(),
            model_id: "test".into(),
            revision: None,
        },
        reasoning_effort: None,
        service_tier: None,
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        skill_instructions: vec![],
    };
    let mut input = ProviderInput {
        audit_text: "old audit".into(),
        system_text: Some("No tools. Return JSON only.".into()),
        messages: vec![ProviderMessage {
            role: ProviderMessageRole::User,
            text: r#"[{"kind":"task_prompt","content":"old"}]"#.into(),
            images: vec![ProviderImage {
                context_item_id: "old-picture".into(),
                context_text: "preloaded excerpt".into(),
                data_url: "not an image".into(),
            }],
        }],
    };
    decorate_submission(&mut input, &submission, "Synthetic accepted task policy").unwrap();
    assert_eq!(input.audit_text, submission.text);
    let messages =
        super::super::super::paper_reading_shared_transport::provider_messages(input, false)
            .unwrap();
    let research_agent_core::model::ModelMessage::User { content: parts, .. } =
        messages.last().unwrap()
    else {
        panic!()
    };
    assert!(
        matches!(&parts[0], research_agent_core::model::ContentPart::Text(text) if text == &submission.text)
    );
    assert_eq!(messages.len(), 2);
    assert_eq!(
        parts.len(),
        1,
        "source bodies and figures must be read on demand"
    );
}

#[test]
fn typed_task_skill_is_versioned_and_cannot_bypass_disabled_policy() {
    let root = tempfile::tempdir().unwrap();
    crate::agent::skills::bundled::install_defaults(root.path()).unwrap();
    let prompt = accepted_prompt();
    let mut input = research_agent_core::submission::SubmissionInput {
        remote_compaction_v2: None,
        text: prompt.text,
        user_input: prompt.user_input,
        context: grants("portrait"),
        model: research_agent_core::protocol::ModelIdentity {
            provider_id: "test".into(),
            model_id: "test".into(),
            revision: None,
        },
        reasoning_effort: None,
        service_tier: None,
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        skill_instructions: vec![],
    };
    super::super::bind_task_skill(root.path(), "portrait", &mut input).unwrap();
    assert!(input.text.starts_with("按照 @generate-paper-portrait"));
    assert!(input.text.contains("paper_portrait_v1 JSON 对象"));
    assert!(!input.text.contains("quoteExact"));
    assert!(!input.text.contains("Do not invoke any tool"));
    research_agent_core::user_input::validate(&input.user_input, &input.text).unwrap();
    assert!(
        input
            .skills
            .iter()
            .any(|skill| skill.explicit && skill.descriptor.name == "generate-paper-portrait")
    );
    assert!(
        input
            .skill_instructions
            .iter()
            .any(|skill| skill.contents.contains("coverage is a string"))
    );
    let contents = &input
        .skill_instructions
        .iter()
        .find(|skill| skill.name == "generate-paper-portrait")
        .unwrap()
        .contents;
    let example = contents
        .split_once("```json\n")
        .unwrap()
        .1
        .split_once("\n```")
        .unwrap()
        .0;
    let value: crate::ai::candidate_contracts::CandidatePayload =
        serde_json::from_str(example).unwrap();
    assert_eq!(value.kind(), "paper_portrait_v1");
    crate::agent::config::skills::set_enabled(
        root.path(),
        ".agents/skills/generate-paper-portrait/SKILL.md",
        false,
    )
    .unwrap();
    assert!(super::super::bind_task_skill(root.path(), "portrait", &mut input).is_err());
}

#[test]
fn runtime_prompt_override_is_persisted_for_policy_input() {
    let root = tempfile::tempdir().unwrap();
    let before = crate::system_documents::read(root.path(), "structured-task").unwrap();
    let content = format!("{}\nCurrent runtime prompt rule.\n", before.content);
    let saved = crate::system_documents::write(
        root.path(),
        &before.id,
        &before.revision,
        &content,
        &before.baseline,
    )
    .unwrap();
    assert_ne!(saved.content, saved.baseline);
    let task = crate::system_documents::read(root.path(), "structured-task").unwrap();
    let contract =
        crate::system_documents::read(root.path(), "structured-reading-contract").unwrap();
    assert_eq!(task.content, content);
    assert!(task.content.contains("Current runtime prompt rule."));
    assert!(!contract.content.is_empty());
}

#[test]
fn context_classification_passes_owned_resource_facts_to_prompt_policy() {
    let sources = grants("portrait");
    assert_eq!(
        prompt_resource(&sources[0]),
        Some(PromptResourceV1::Paper {
            resource_id: "paper".into(),
            attachment_id: Some("pdf-1".into())
        })
    );
    assert_eq!(prompt_resource(&sources[1]), Some(PromptResourceV1::Other));
    let catalog = grant(
        "catalog",
        json!({"kind":"templateCatalog","resourceId":"catalog","access":"readOnly"}),
        Some("templateCatalog"),
    );
    assert_eq!(
        prompt_resource(&catalog),
        Some(PromptResourceV1::TemplateCatalog)
    );
    let mut invalid = sources[0].clone();
    invalid.anchor = None;
    assert_eq!(prompt_resource(&invalid), None);
}
