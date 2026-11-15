//! Closed application-owned document catalog; source paths are not filesystem grants.
pub struct Definition {
    pub id: &'static str,
    pub asset_path: &'static str,
    pub name: &'static str,
    pub workflow: &'static str,
    pub skill_path: Option<&'static str>,
    pub embedded: &'static str,
}
macro_rules! skill {
    ($id:literal,$name:literal,$workflow:literal,$path:literal) => {
        Definition {
            id: $id,
            asset_path: concat!("skills/", $path),
            name: $name,
            workflow: $workflow,
            skill_path: Some(concat!(".agents/skills/", $path)),
            embedded: include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../agent-resources/skills/",
                $path
            )),
        }
    };
}
pub const DEFINITIONS: &[Definition] = &[
    Definition {
        id: "structured-task",
        asset_path: "prompts/structured-task.md",
        name: "阅读任务提示词.md",
        workflow: "shared",
        skill_path: None,
        embedded: include_str!("../../../../../../packages/core/prompts/structured-task.md"),
    },
    Definition {
        id: "structured-reading-contract",
        asset_path: "prompts/structured-reading-contract.md",
        name: "结构化阅读格式.md",
        workflow: "reading",
        skill_path: None,
        embedded: include_str!(
            "../../../../../../packages/core/prompts/structured-reading-contract.md"
        ),
    },
    skill!(
        "generate-paper-portrait",
        "文献画像技能 · SKILL.md",
        "portrait",
        "generate-paper-portrait/SKILL.md"
    ),
    skill!(
        "edit-structured-resources",
        "结构化资源技能 · SKILL.md",
        "shared",
        "edit-structured-resources/SKILL.md"
    ),
    skill!(
        "portrait-reference",
        "画像格式 · portrait.md",
        "portrait",
        "edit-structured-resources/references/portrait.md"
    ),
    skill!(
        "reading-reference",
        "阅读格式 · reading.md",
        "reading",
        "edit-structured-resources/references/reading.md"
    ),
    skill!(
        "templates-reference",
        "模板格式 · templates.md",
        "shared",
        "edit-structured-resources/references/templates.md"
    ),
];
pub fn definition(id: &str) -> Result<&'static Definition, String> {
    DEFINITIONS
        .iter()
        .find(|d| d.id == id)
        .ok_or("内置文档不存在".into())
}
pub fn id_from_path(path: &str) -> Result<&'static str, String> {
    let parts = research_agent_core::text_patch::address::parse(path)?;
    if parts.first().map(String::as_str) != Some("agents") {
        return Err("找不到文件对象".into());
    }
    DEFINITIONS
        .iter()
        .find(|definition| {
            parts[1..]
                .iter()
                .map(String::as_str)
                .eq(definition.asset_path.split('/'))
        })
        .map(|definition| definition.id)
        .ok_or("找不到文件对象".into())
}
pub fn path(id: &str) -> Result<String, String> {
    let definition = definition(id)?;
    Ok(research_agent_core::text_patch::address::object(
        research_agent_core::text_patch::address::SourceRoot::Agents,
        &definition
            .asset_path
            .split('/')
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    ))
}
pub fn validate(id: &str, text: &str) -> Result<(), String> {
    definition(id)?;
    if text.trim().is_empty() || text.len() > 131_072 || text.contains('\0') {
        return Err("内置文档需为非空 UTF-8 文本，最多 128 KiB".into());
    }
    if matches!(id, "generate-paper-portrait" | "edit-structured-resources") {
        let d = definition(id)?;
        let parsed = crate::agent::skills::parse_document(
            "application",
            d.skill_path.unwrap(),
            id,
            text.as_bytes(),
        )
        .map_err(|_| "技能元数据无效，请保留 name 与 description")?;
        if parsed.descriptor.name != id {
            return Err("不能修改内置技能的 name 标识".into());
        }
    }
    if matches!(id, "structured-task" | "structured-reading-contract") {
        let template = research_agent_core::prompt_template::Template::parse(text)
            .map_err(|e| e.to_string())?;
        let bindings = if id == "structured-task" {
            vec![("task", ""), ("contract", ""), ("preferences", "")]
        } else {
            vec![("schema", "")]
        };
        template
            .render(bindings)
            .map_err(|e| format!("请保留提示词变量：{e}"))?;
    }
    Ok(())
}
