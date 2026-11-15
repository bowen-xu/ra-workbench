//! SKILL.md parsing only; filesystem discovery and authorization belong to host.
// Frontmatter framing and single-line metadata adapted from OpenAI Codex
// core-skills/src/loader.rs, e363b08c9175ac1cbe5893615dd2cb9ddf95043b.
// Copyright 2025 OpenAI. Apache-2.0; see packages/agent-core/third-party/codex/.
use research_agent_core::skills::{LoadedSkill, SkillDescriptor, SkillIdentity};
use serde::Deserialize;
use sha2::{Digest, Sha256};
#[path = "skills/application.rs"]
pub mod application;
#[path = "skills/bundled.rs"]
pub(crate) mod bundled;
#[path = "skills/discovery.rs"]
pub mod discovery;
#[path = "skills/files.rs"]
pub mod files;
#[path = "skills/import.rs"]
pub(crate) mod import;
#[path = "skills/injection.rs"]
pub(crate) mod injection;
#[path = "skills/packages.rs"]
pub(crate) mod packages;
#[path = "skills/policy.rs"]
mod policy;
#[path = "skills/resources.rs"]
pub mod resources;
#[path = "skills/steering.rs"]
pub(crate) mod steering;
#[path = "skills/tools.rs"]
pub mod tools;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SkillParseError {
    #[error("skill document exceeds its byte limit")]
    Budget,
    #[error("skill document must be UTF-8")]
    Encoding,
    #[error("skill frontmatter or metadata is invalid")]
    Invalid,
}
#[derive(Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: String,
}
#[derive(Debug, PartialEq, Eq)]
pub struct ParsedSkill {
    pub descriptor: SkillDescriptor,
    pub loaded: LoadedSkill,
}
/// Bytes must already be authorized and bounded by native IO. This second bound
/// protects standalone callers. Unknown metadata is ignored, never a capability;
/// !include, properties and tag constructors are not enabled.
pub fn parse_document(
    environment: &str,
    source: &str,
    default_name: &str,
    bytes: &[u8],
) -> Result<ParsedSkill, SkillParseError> {
    if bytes.len() > 1024 * 1024 {
        return Err(SkillParseError::Budget);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| SkillParseError::Encoding)?;
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().ok_or(SkillParseError::Invalid)?;
    if first.trim() != "---" {
        return Err(SkillParseError::Invalid);
    }
    let start = first.len();
    let mut offset = start;
    let mut closing = None;
    for line in lines {
        if offset - start > 65536 {
            return Err(SkillParseError::Budget);
        }
        if line.trim() == "---" {
            closing = Some((offset, offset + line.len()));
            break;
        }
        offset += line.len();
    }
    let (end, body_start) = closing.ok_or(SkillParseError::Invalid)?;
    if end - start > 65536 {
        return Err(SkillParseError::Budget);
    }
    let metadata: Frontmatter = parse_yaml(&text[start..end])?;
    let single_line = |value: &str| value.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = metadata
        .name
        .as_deref()
        .map(single_line)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| single_line(default_name));
    let identity = SkillIdentity {
        environment_id: environment.into(),
        source_id: source.into(),
        revision: format!("{:x}", Sha256::digest(bytes)),
    };
    let descriptor = SkillDescriptor {
        identity: identity.clone(),
        name,
        description: single_line(&metadata.description),
        allow_implicit_invocation: true,
    };
    descriptor
        .validate()
        .map_err(|_| SkillParseError::Invalid)?;
    let body = text[body_start..].to_owned();
    if body.trim().is_empty() || body.contains('\0') {
        return Err(SkillParseError::Invalid);
    }
    Ok(ParsedSkill {
        descriptor,
        loaded: LoadedSkill { identity, body },
    })
}

pub(super) fn parse_yaml<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, SkillParseError> {
    if text.len() > 65536 {
        return Err(SkillParseError::Budget);
    }
    let options = serde_saphyr::options! {
        reject_unsupported_tags: true,
        strict_booleans: true,
        budget: serde_saphyr::budget! {
            max_depth: 16, max_nodes: 2048, max_events: 8192,
            max_aliases: 0, max_anchors: 0, max_merge_keys: 0,
            max_documents: 1, max_total_scalar_bytes: 65536,
        },
    };
    serde_saphyr::from_str_with_options(text, options).map_err(|_| SkillParseError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(text: &str) -> Result<ParsedSkill, SkillParseError> {
        parse_document(
            "project",
            "proofread/SKILL.md",
            "proofread",
            text.as_bytes(),
        )
    }
    #[test]
    fn folded_metadata_crlf_body_and_exact_version_are_preserved() {
        let text = "---\r\ndescription: >-\r\n  Check prose\r\n  for clarity.\r\n---\r\nUse exact terminology.\r\n";
        let first = parse(text).unwrap();
        assert_eq!(first.descriptor.name, "proofread");
        assert_eq!(first.descriptor.description, "Check prose for clarity.");
        assert_eq!(first.loaded.body, "Use exact terminology.\r\n");
        assert_eq!(first.loaded.identity, first.descriptor.identity);
        assert_ne!(
            first.descriptor.identity.revision,
            parse(&format!("{text}\n"))
                .unwrap()
                .descriptor
                .identity
                .revision
        );
    }
    #[test]
    fn rejects_duplicate_missing_or_expanding_metadata_without_echoing_values() {
        for text in [
            "description: x\nbody",
            "---\ndescription: x",
            "---\nname: x\n---\nbody",
            "---\ndescription: first\ndescription: second\n---\nbody",
            "---\ndescription: &a private\nname: *a\n---\nbody",
            "---\ndescription: !include private-file\n---\nbody",
            "---\ndescription: [PRIVATE, VALUE]\n---\nbody",
            "---\ndescription: x\n---\n",
        ] {
            let error = parse(text).unwrap_err();
            assert_eq!(error, SkillParseError::Invalid);
            assert!(!error.to_string().contains("PRIVATE"));
        }
        assert_eq!(
            parse_document("p", "s", "n", &[0xff]),
            Err(SkillParseError::Encoding)
        );
        assert_eq!(
            parse_document("p", "s", "n", &vec![b' '; 1024 * 1024 + 1]),
            Err(SkillParseError::Budget)
        );
    }
    #[test]
    fn unrelated_metadata_does_not_register_permissions_or_tools() {
        let parsed = parse("---\nname: proofread\ndescription: Review writing\nallowed-tools: [shell, network]\n---\nRead carefully.").unwrap();
        let metadata = serde_json::to_string(&parsed.descriptor).unwrap();
        assert!(!metadata.contains("shell"));
        assert!(!metadata.contains("network"));
    }

    #[test]
    fn nested_unknown_metadata_and_large_headers_still_obey_budgets() {
        let nested = format!(
            "---\ndescription: Valid\nextra: {}0{}\n---\nBody",
            "[".repeat(40),
            "]".repeat(40)
        );
        assert_eq!(parse(&nested), Err(SkillParseError::Invalid));
        let large = format!("---\ndescription: {}\n---\nBody", "a".repeat(65536));
        assert_eq!(parse(&large), Err(SkillParseError::Budget));
    }
}
