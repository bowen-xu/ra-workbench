//! Codex-compatible agents/openai.yaml invocation policy. Other metadata grants
//! no tools or dependencies. The complete sidecar bytes participate in revision.
use super::{ParsedSkill, SkillParseError, parse_yaml};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Default, Deserialize)]
struct Metadata {
    policy: Option<Policy>,
}
#[derive(Deserialize)]
struct Policy {
    allow_implicit_invocation: Option<bool>,
}

pub(super) fn apply(parsed: &mut ParsedSkill, bytes: &[u8]) -> Result<(), SkillParseError> {
    if bytes.len() > 65536 {
        return Err(SkillParseError::Budget);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| SkillParseError::Encoding)?;
    let metadata: Metadata = parse_yaml(text)?;
    parsed.descriptor.allow_implicit_invocation = metadata
        .policy
        .and_then(|policy| policy.allow_implicit_invocation)
        .unwrap_or(true);
    let mut hash = Sha256::new();
    hash.update(b"research-assistant:skill-with-policy:v1\0");
    hash.update(parsed.descriptor.identity.revision.as_bytes());
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    parsed.descriptor.identity.revision = format!("{:x}", hash.finalize());
    parsed.loaded.identity = parsed.descriptor.identity.clone();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn skill() -> ParsedSkill {
        super::super::parse_document(
            "paper",
            "review/SKILL.md",
            "review",
            b"---\ndescription: Review\n---\nBody",
        )
        .unwrap()
    }
    #[test]
    fn policy_disables_implicit_loading_and_versions_even_non_policy_metadata() {
        let mut first = skill();
        let original = first.descriptor.identity.revision.clone();
        apply(&mut first, b"policy:\n  allow_implicit_invocation: false\n").unwrap();
        assert!(!first.descriptor.allow_implicit_invocation);
        assert_ne!(first.descriptor.identity.revision, original);
        assert_eq!(first.loaded.identity, first.descriptor.identity);
        let mut second = skill();
        apply(
            &mut second,
            b"policy:\n  allow_implicit_invocation: false\n# changed\n",
        )
        .unwrap();
        assert_ne!(first.loaded.identity, second.loaded.identity);
        let mut default = skill();
        apply(&mut default, b"policy: {}\ndependencies: {tools: []}").unwrap();
        assert!(default.descriptor.allow_implicit_invocation);
    }
    #[test]
    fn invalid_or_expanding_policy_is_rejected() {
        for bytes in [
            b"policy: {allow_implicit_invocation: yes}".as_slice(),
            b"policy: {allow_implicit_invocation: true, allow_implicit_invocation: false}",
            b"policy: &policy {}\nother: *policy",
            b"policy: !include secret",
            &[0xff],
        ] {
            assert!(apply(&mut skill(), bytes).is_err());
        }
    }
}
