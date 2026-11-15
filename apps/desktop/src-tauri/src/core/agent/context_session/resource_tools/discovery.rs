//! Bounded path discovery adapted from Apache-2.0 Codex e363b08c9175ac1cbe5893615dd2cb9ddf95043b:
//! file-search's limited results and shell's `rg --files -g` / output budget contract.
//! Copyright 2025 OpenAI. Application IDs replace filesystem roots; no shell or symlink traversal.
use super::*;
use regex::{Regex, RegexBuilder};

const PAGE_BYTES: usize = 32 * 1024;
#[derive(Default)]
pub(super) struct Filters {
    pub resource_id: Option<String>,
    pub name_query: Option<String>,
    pub path_glob: Option<String>,
    pub resource_type: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    resource_id: Option<String>,
    name_query: Option<String>,
    path_glob: Option<String>,
    resource_type: Option<String>,
    #[serde(default)]
    offset: usize,
    #[serde(default = "page_limit")]
    limit: usize,
}
fn page_limit() -> usize {
    20
}
impl Args {
    fn filters(&self) -> Filters {
        Filters {
            resource_id: self.resource_id.clone(),
            name_query: self.name_query.clone(),
            path_glob: self.path_glob.clone(),
            resource_type: self.resource_type.clone(),
        }
    }
}
pub(super) fn filter_properties() -> Vec<(&'static str, Value)> {
    vec![
        (
            "resourceId",
            json!({"type":"string","minLength":1,"maxLength":1024,"description":"Authorized literature/project object ID; never a local filesystem path."}),
        ),
        (
            "nameQuery",
            json!({"type":"string","minLength":1,"maxLength":512,"description":"Case-insensitive literal substring in current display name or project-relative path; does not read content."}),
        ),
        (
            "pathGlob",
            json!({"type":"string","minLength":1,"maxLength":512,"description":"Case-insensitive name/path glob: * matches within a component, ** crosses /, ? matches one character. No regex, braces or character classes. A pattern without / matches the basename; e.g. *.pdf or sections/**/*.tex."}),
        ),
        (
            "resourceType",
            json!({"type":"string","enum":["paper","note","manuscriptFile","paperMetadata","paperPortrait","portrait","structuredReading","readingTemplate","templateCatalog"],"description":"paper means PDF backed by Docling; portrait, structuredReading and readingTemplate are editable structured objects."}),
        ),
    ]
}
pub(super) fn definition(find: bool) -> ToolDefinition {
    let mut properties = json!({"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":20}});
    for (key, schema) in filter_properties() {
        properties[key] = schema;
    }
    ToolDefinition {
        namespace: None,
        name: if find {"research_find_sources"} else {"research_list_sources"}.into(),
        description: if find {
            "Find current readable resource NAMES/PATHS without loading bodies. REQUIRED: supply at least one of nameQuery, pathGlob or resourceType. resourceId alone is NOT a search filter and will be rejected. To discover reading templates, use resourceType: readingTemplate and optionally resourceId to scope the catalog. To browse a catalog without a search filter, use research_list_sources with resourceId instead. Never call find with empty arguments. AND filters, stable name order, up to 20 compact entries and 32 KiB per page; follow nextOffset only when needed. Unreadable entries are omitted. Use returned sourceId with read/search tools."
        } else {
            "List current readable resources, optionally narrowed by resourceId, nameQuery, pathGlob or resourceType. Up to 20 compact names/identities/permissions and 32 KiB per page; follow nextOffset. Prefer research_find_sources for targeted discovery, not paging the entire inventory. No bodies or local authorization anchors are returned."
        }.into(),
        parameters:json!({"type":"object","properties":properties,"additionalProperties":false}),
    }
}
fn parse(call: &ToolCall) -> Result<Args, RunError> {
    let args: Args = serde_json::from_value(call.arguments.clone()).map_err(|_|RunError::ToolArguments("Provide optional resourceId, nameQuery, pathGlob, resourceType, offset and limit (1–20). No extra fields.".into()))?;
    if !(1..=20).contains(&args.limit) {
        return Err(RunError::ToolArguments(
            "limit must be 1–20; follow nextOffset for more results.".into(),
        ));
    }
    validate_filters(&args.filters())?;
    if call.name == "research_find_sources"
        && args.name_query.is_none()
        && args.path_glob.is_none()
        && args.resource_type.is_none()
    {
        return Err(RunError::ToolArguments("Find requires nameQuery, pathGlob or resourceType. Use research_list_sources for an unfiltered page.".into()));
    }
    Ok(args)
}
pub(super) fn validate(call: &ToolCall) -> Result<(), RunError> {
    parse(call).map(|_| ())
}
pub(super) fn validate_filters(filters: &Filters) -> Result<(), RunError> {
    for (name, value, max) in [
        ("resourceId", &filters.resource_id, 1024),
        ("nameQuery", &filters.name_query, 512),
        ("pathGlob", &filters.path_glob, 512),
    ] {
        if value
            .as_ref()
            .is_some_and(|v| v.trim().is_empty() || v.len() > max || v.contains('\0'))
        {
            return Err(RunError::ToolArguments(format!(
                "{name} must be non-empty, without NUL, and at most {max} UTF-8 bytes."
            )));
        }
    }
    if filters.resource_type.as_ref().is_some_and(|v| {
        !matches!(
            v.as_str(),
            "paper"
                | "note"
                | "manuscriptFile"
                | "paperMetadata"
                | "paperPortrait"
                | "portrait"
                | "structuredReading"
                | "readingTemplate"
                | "templateCatalog"
        )
    }) {
        return Err(RunError::ToolArguments(
            "resourceType must be paper, note, manuscriptFile, paperMetadata or paperPortrait."
                .into(),
        ));
    }
    if let Some(pattern) = &filters.path_glob {
        glob(pattern).map_err(|_|RunError::ToolArguments("Invalid pathGlob; use *, ** and ? in names or relative paths, without .., absolute paths, braces or character classes.".into()))?;
    }
    Ok(())
}
fn glob(pattern: &str) -> Result<Regex, Error> {
    if pattern.starts_with('/')
        || pattern.contains('\\')
        || pattern.split('/').any(|p| p == "..")
        || pattern.contains(['[', ']', '{', '}'])
    {
        return Err(Error::InvalidInput);
    }
    let mut expression = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                    expression.push_str("(?:.*/)?");
                } else {
                    expression.push_str(".*");
                }
            }
            '*' => expression.push_str("[^/]*"),
            '?' => expression.push_str("[^/]"),
            c => expression.push_str(&regex::escape(&c.to_string())),
        }
    }
    expression.push('$');
    RegexBuilder::new(&expression)
        .case_insensitive(true)
        .dot_matches_new_line(true)
        .size_limit(1024 * 1024)
        .build()
        .map_err(|_| Error::InvalidInput)
}
fn identity(entry: &Entry) -> Value {
    entry
        .resource
        .as_ref()
        .and_then(|r| serde_json::to_value(r).ok())
        .unwrap_or_else(|| json!({"kind":"historical"}))
}
fn resource_type(entry: &Entry) -> Value {
    entry
        .identity
        .anchor
        .as_deref()
        .and_then(|anchor| serde_json::from_str::<Value>(anchor).ok())
        .and_then(|anchor| anchor.get("resourceKind").cloned())
        .unwrap_or_else(|| identity(entry)["kind"].clone())
}
pub(super) fn name(entry: &Entry) -> String {
    if let Some(label) = entry
        .identity
        .anchor
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .and_then(|value| value["label"].as_str().map(str::to_owned))
    {
        return label;
    }
    if let Some(Resource::ManuscriptFile { path, .. }) = &entry.resource {
        return path.clone();
    }
    serde_json::from_str::<Value>(&entry.identity.text)
        .ok()
        .and_then(|v| v["label"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "Historical source".into())
}
pub(super) fn filter(
    tools: &ResourceTools,
    entries: &mut Vec<Entry>,
    filters: &Filters,
) -> Result<(), Error> {
    if let Some(owner) = &filters.resource_id {
        let authorized = entries.iter().any(|entry| {
            entry.owner.as_ref() == Some(owner) || entry.identity.resource_id == *owner
        }) || tools.record.input.context.iter().any(|s| {
            resource_grants::resource(s)
                .and_then(|r| serde_json::to_value(r).ok())
                .is_some_and(|r| r["resourceId"] == *owner)
        });
        if !authorized {
            return Err(Error::SourceUnavailable);
        }
        entries.retain(|entry| {
            entry.owner.as_ref() == Some(owner) || entry.identity.resource_id == *owner
        });
    }
    let query = filters.name_query.as_ref().map(|q| q.to_lowercase());
    let pattern = filters.path_glob.as_ref().map(|p| glob(p)).transpose()?;
    entries.retain(|entry| {
        if filters
            .resource_type
            .as_ref()
            .is_some_and(|kind| resource_type(entry) != *kind)
        {
            return false;
        }
        let name = name(entry);
        if query
            .as_ref()
            .is_some_and(|q| !name.to_lowercase().contains(q))
        {
            return false;
        }
        let candidate = if filters.path_glob.as_ref().is_some_and(|p| !p.contains('/')) {
            name.rsplit('/').next().unwrap_or(&name)
        } else {
            &name
        };
        pattern.as_ref().is_none_or(|p| p.is_match(candidate))
    });
    Ok(())
}
fn row(entry: &Entry) -> Value {
    let original = name(entry);
    let shortened: String = original.chars().take(256).collect();
    let resource = identity(entry);
    let mut row = json!({"sourceId":entry.identity.source_id,"kind":entry.identity.kind,"resourceType":resource_type(entry),"resourceId":entry.identity.resource_id,"ownerResourceId":entry.owner,"name":shortened,"nameTruncated":original!=shortened,"access":entry.identity.anchor.as_deref().and_then(|a|serde_json::from_str::<Value>(a).ok()).map(|a|a["contextAccess"].clone()).unwrap_or(json!("readOnly"))});
    if let Some(address) = super::source_link::address(entry) {
        row["sourceUrl"] = json!(address);
    }
    for key in ["path", "attachmentId", "pageIndex"] {
        if !resource[key].is_null() {
            row[key] = resource[key].clone();
        }
    }
    if let Some(id) = super::file_handle::id(entry) {
        row["fileId"] = json!(id);
    }
    row
}
pub(super) fn evaluate(
    tools: &ResourceTools,
    _db: &Connection,
    mut entries: Vec<Entry>,
    call: &ToolCall,
) -> Result<Value, Error> {
    let args = parse(call).map_err(|_| Error::InvalidInput)?;
    filter(tools, &mut entries, &args.filters())?;
    entries.sort_by_cached_key(|e| (name(e).to_lowercase(), e.identity.source_id.clone()));
    if args.offset > entries.len() {
        return Err(Error::InvalidInput);
    }
    let mut result = json!({"turnId":tools.record.turn_id,"total":entries.len(),"nextOffset":null,"sources":[],"outputByteLimit":PAGE_BYTES});
    let mut rows = Vec::new();
    for entry in entries.iter().skip(args.offset).take(args.limit) {
        rows.push(row(entry));
        result["sources"] = json!(rows);
        let next = args.offset + rows.len();
        result["nextOffset"] = if next < entries.len() {
            json!(next)
        } else {
            Value::Null
        };
        if result.to_string().len() > PAGE_BYTES {
            rows.pop();
            if rows.is_empty() {
                return Err(Error::Budget);
            }
            result["sources"] = json!(rows);
            result["nextOffset"] = json!(args.offset + rows.len());
            break;
        }
    }
    Ok(result)
}

/// Preserve whole matches and a continuation position instead of truncating JSON.
pub(super) fn bound_matches(mut value: Value, offset: usize) -> Result<Value, Error> {
    value["outputByteLimit"] = json!(PAGE_BYTES);
    if let Some(matches) = value["matches"].as_array_mut() {
        for matched in matches {
            matched
                .as_object_mut()
                .ok_or(Error::InvalidInput)?
                .remove("anchor");
        }
    }
    while value.to_string().len() > PAGE_BYTES {
        let matches = value["matches"].as_array_mut().ok_or(Error::InvalidInput)?;
        matches.pop();
        if matches.is_empty() {
            return Err(Error::Budget);
        }
        value["nextOffset"] = json!(offset + matches.len());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn glob_contract_matches_zero_or_more_nested_directories() {
        let nested = glob("sections/**/*.tex").unwrap();
        assert!(nested.is_match("sections/methods.tex"));
        assert!(nested.is_match("sections/parts/METHODS.tex"));
        assert!(!nested.is_match("other/methods.tex"));
        assert!(
            !glob("sections/*.tex")
                .unwrap()
                .is_match("sections/parts/a.tex")
        );
        assert!(glob("?.tex").unwrap().is_match("文.tex"));
        for invalid in ["../*.tex", "/private/*", "[ab].tex", "{a,b}.tex"] {
            assert!(glob(invalid).is_err());
        }
    }
    #[test]
    fn content_result_byte_budget_preserves_whole_matches_and_next_offset() {
        let matches:Vec<_>=(0..20).map(|i|json!({"sourceId":format!("source-{i}"),"snippet":"文".repeat(1000),"anchor":"never sent"})).collect();
        let value =
            bound_matches(json!({"total":20,"nextOffset":null,"matches":matches}), 0).unwrap();
        let length = value["matches"].as_array().unwrap().len();
        assert!(length > 0 && length < 20);
        assert_eq!(value["nextOffset"], length);
        assert!(value.to_string().len() <= PAGE_BYTES);
        assert!(!value.to_string().contains("anchor"));
        assert_eq!(
            value["matches"][0]["snippet"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            1000
        );
    }
}
