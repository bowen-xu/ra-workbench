use super::*;
use crate::ai::{
    generation_contracts::ProviderImage,
    generation_support::{ProviderInput, ProviderMessage, ProviderMessageRole},
    structured_generation_support::{decorate_submission, prompt_resource},
};
use research_agent_core::submission::ContextSourceKind;
use research_agent_core::submission::ContextText;
use research_agent_core::user_input::MentionTarget;
use research_literature_library_native::analysis_workflow::{
    submission::submission_scope, task_prompt::render_current,
};
use research_workbench_native_sdk::{
    analysis_workflow::{AnalysisWorkflowError, prompt::PaperTaskPrompt},
    candidate_contracts::AiOverviewProfileContext,
};
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

fn render(
    context: &[ContextText],
    kind: &str,
    profile: Option<&AiOverviewProfileContext>,
    preferences: &str,
) -> Result<PaperTaskPrompt, AnalysisWorkflowError> {
    render_current(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../.dependencies/sdk/web/core/prompts/structured-task.md"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../.dependencies/sdk/web/core/prompts/structured-reading-contract.md"
        )),
        context
            .iter()
            .filter_map(|source| prompt_resource(source).map(|resource| (source, resource))),
        kind,
        profile,
        preferences,
    )
}

#[test]
fn readable_prompt_binds_exact_authorized_files_without_context_bookkeeping() {
    let prompt = render(
        &grants("structuredReading"),
        "structuredReading",
        None,
        "含 emoji 🧪 和 {{ literal }}",
    )
    .unwrap();
    assert!(prompt.text.starts_with(
        "阅读以下文件：\n- [source://library/paper/pdf-1](source://library/paper/pdf-1)\n"
    ));
    assert!(
        prompt
            .text
            .contains("写入到 [source://library/paper/target](source://library/paper/target)")
    );
    assert!(prompt.text.contains("\"kind\": \"paper_overview_unit_v1\""));
    assert!(!prompt.text.contains("system_context"));
    assert!(!prompt.text.contains("context snapshot"));
    assert!(prompt.text.contains("含 emoji 🧪 和 {{ literal }}"));
    research_agent_core::user_input::validate(&prompt.user_input, &prompt.text).unwrap();
    let UserInput::Text { text_elements, .. } = &prompt.user_input[0] else {
        panic!()
    };
    for element in text_elements {
        let marked = String::from_utf16(
            &prompt.text.encode_utf16().collect::<Vec<_>>()[element.start_utf16..element.end_utf16],
        )
        .unwrap();
        let UserInput::Mention {
            name,
            target:
                MentionTarget::File {
                    source_id,
                    revision,
                    ..
                },
        } = &prompt.user_input[element.item_index]
        else {
            panic!()
        };
        assert_eq!(marked, format!("[{name}]({name})"));
        assert!(source_id.starts_with("grant:"));
        assert!(revision.starts_with("frozen:"));
    }
}

#[test]
fn portrait_has_its_contract_and_separate_template_proposal_target() {
    let mut sources = grants("portrait");
    sources.push(grant(
        "catalog",
        json!({"kind":"templateCatalog","resourceId":"catalog","access":"readOnly"}),
        Some("templateCatalog"),
    ));
    let prompt = render(&sources, "portrait", None, "").unwrap();
    assert!(prompt.text.contains("resourceType: readingTemplate"));
    assert!(prompt.text.contains("不要只传 resourceId 调用 find"));
    assert!(prompt.text.contains("遵循所引用画像技能"));
    assert!(prompt.text.contains(
        "本文模板建议写入到 [source://library/paper/template](source://library/paper/template)"
    ));
    assert!(prompt.text.contains("Keep／Undo"));
    assert!(!prompt.text.contains("No tools"));
    research_agent_core::user_input::validate(&prompt.user_input, &prompt.text).unwrap();
}

#[test]
fn missing_target_cannot_be_turned_into_a_guessed_write_address() {
    assert!(render(&[], "structuredReading", None, "").is_err());
}

#[test]
fn outgoing_provider_user_message_is_the_admitted_task_not_the_legacy_envelope() {
    let grants = grants("structuredReading");
    let prompt = render(&grants, "structuredReading", None, "").unwrap();
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
    let scope =
        submission_scope("paper_overview", &["paper".into()], vec!["pdf-1".into()]).unwrap();
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
    decorate_submission(&mut input, &submission, &scope.system_text).unwrap();
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
    let prompt = render(&grants("portrait"), "portrait", None, "配置 🧪").unwrap();
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
    research_literature_library_native::analysis_workflow::candidate::decode_candidate_value(
        serde_json::from_str(example).unwrap(),
        &std::collections::HashSet::new(),
    )
    .expect("the skill's complete JSON example must satisfy the actual domain decoder");
    crate::agent::config::skills::set_enabled(
        root.path(),
        ".agents/skills/generate-paper-portrait/SKILL.md",
        false,
    )
    .unwrap();
    assert!(super::super::bind_task_skill(root.path(), "portrait", &mut input).is_err());
}

#[test]
fn runtime_prompt_override_is_rendered_before_review_keep() {
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
    let sources = grants("portrait");
    let result = render_current(
        &task.content,
        &contract.content,
        sources
            .iter()
            .filter_map(|source| prompt_resource(source).map(|resource| (source, resource))),
        "portrait",
        None,
        "",
    )
    .unwrap();
    assert!(result.text.contains("Current runtime prompt rule."));
}

#[test]
fn task_cannot_promote_a_readonly_target_to_a_write_request() {
    for kind in ["portrait", "structuredReading"] {
        let mut sources = grants(kind);
        let mut anchor: Value =
            serde_json::from_str(sources[1].anchor.as_deref().unwrap()).unwrap();
        anchor["contextAccess"] = json!("readOnly");
        sources[1].anchor = Some(anchor.to_string());
        let error = render(&sources, kind, None, "").err().unwrap();
        assert!(error.to_string().contains("读写权限"));
        sources.remove(1);
        assert!(render(&sources, kind, None, "").is_err());
    }
}
