//! App-owned MCP process configuration.
//!
//! Adapted from Codex MCP server configuration loading. The host reads and
//! authorizes the configuration root; this module only validates a bounded
//! document and produces a host-owned `ProcessSpec`. Credentials are not part
//! of this document and environment inheritance is disabled.

use super::process::ProcessSpec;
use crate::agent::instructions::directory::Directory;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_SERVERS: usize = 64;
const MAX_ARGUMENTS: usize = 64;
const MAX_ENVIRONMENT: usize = 64;
const MAX_TEXT_BYTES: usize = 4096;
const MAX_DOCUMENT_BYTES: usize = 256 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("MCP configuration is invalid")]
    Invalid,
    #[error("MCP configuration exceeds its budget")]
    Budget,
    #[error("MCP configuration could not be read safely")]
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredServer {
    pub server_id: String,
    pub environment_id: String,
    pub revision: String,
    pub transport: Transport,
    pub environment: BTreeMap<OsString, OsString>,
    /// Opaque host-owned reference. The value is never resolved by the
    /// parser and is never exposed to the model or serialized into events.
    pub credential_handle: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    Stdio {
        program: PathBuf,
        arguments: Vec<OsString>,
        directory: PathBuf,
    },
    StreamableHttp {
        url: String,
    },
}

impl ConfiguredServer {
    /// Build the process input only after the host has selected this server.
    /// The returned spec has an explicit empty environment policy.
    pub fn process_spec(&self) -> Option<ProcessSpec> {
        let Transport::Stdio {
            program,
            arguments,
            directory,
        } = &self.transport
        else {
            return None;
        };
        Some(ProcessSpec {
            program: program.clone(),
            arguments: arguments.clone(),
            directory: directory.clone(),
            environment: self.environment.clone(),
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    #[serde(default)]
    servers: BTreeMap<String, Server>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Server {
    environment_id: String,
    revision: String,
    #[serde(default)]
    transport: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    program: Option<String>,
    #[serde(default)]
    arguments: Vec<String>,
    #[serde(default)]
    directory: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    credential_handle: Option<String>,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

fn default_enabled() -> bool {
    true
}

pub fn parse(text: &str) -> Result<Vec<ConfiguredServer>, Error> {
    if text.len() > MAX_DOCUMENT_BYTES {
        return Err(Error::Budget);
    }
    let document: Document = toml::from_str(text).map_err(|_| Error::Invalid)?;
    if document.schema_version != 1 {
        return Err(Error::Invalid);
    }
    if document.servers.len() > MAX_SERVERS {
        return Err(Error::Budget);
    }
    document
        .servers
        .into_iter()
        .map(|(server_id, server)| validate(server_id, server))
        .collect()
}

/// Read the application-owned `agent/mcp.toml` through the same handle-relative
/// no-follow boundary used for Agent instructions. Missing configuration is an
/// empty catalog; present links, non-files and malformed content fail closed.
pub fn read_from_root(root: &Path) -> Result<Option<Vec<ConfiguredServer>>, Error> {
    read_text_from_root(root)?
        .map(|text| parse(&text))
        .transpose()
}

pub(crate) fn read_text_from_root(root: &Path) -> Result<Option<String>, Error> {
    if !root.is_absolute() {
        return Err(Error::Invalid);
    }
    match std::fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::Unavailable),
        Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
            return Err(Error::Invalid);
        }
        Ok(_) => {}
    }
    let canonical = root.canonicalize().map_err(|_| Error::Unavailable)?;
    // macOS exposes /var through /private/var. Resolve ancestor aliases after
    // rejecting a linked configuration root, then use no-follow handles below.
    let directory = Directory::root(&canonical).map_err(|_| Error::Unavailable)?;
    let Some(agent) = directory
        .optional_child(Path::new("agent"))
        .map_err(|_| Error::Unavailable)?
    else {
        return Ok(None);
    };
    let Some(file) = agent
        .config_file("mcp.toml")
        .map_err(|_| Error::Unavailable)?
    else {
        return Ok(None);
    };
    if file.metadata().map_err(|_| Error::Unavailable)?.len() > MAX_DOCUMENT_BYTES as u64 {
        return Err(Error::Budget);
    }
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Unavailable)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(Error::Budget);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| Error::Invalid)?;
    parse(text)?;
    Ok(Some(text.to_owned()))
}

fn validate(server_id: String, server: Server) -> Result<ConfiguredServer, Error> {
    validate_text(&server_id)?;
    validate_text(&server.environment_id)?;
    validate_text(&server.revision)?;
    if server.arguments.len() > MAX_ARGUMENTS || server.environment.len() > MAX_ENVIRONMENT {
        return Err(Error::Budget);
    }
    let transport = server.transport.as_deref().unwrap_or("stdio");
    let arguments = server
        .arguments
        .into_iter()
        .map(|value| {
            validate_text(&value)?;
            Ok(OsString::from(value))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let environment = server
        .environment
        .into_iter()
        .map(|(key, value)| {
            validate_text(&key)?;
            validate_text(&value)?;
            Ok((OsString::from(key), OsString::from(value)))
        })
        .collect::<Result<BTreeMap<_, _>, Error>>()?;
    if let Some(handle) = &server.credential_handle {
        validate_text(handle)?;
    }
    let transport = match transport {
        "stdio" => {
            let program = PathBuf::from(server.program.ok_or(Error::Invalid)?);
            let directory = PathBuf::from(server.directory.ok_or(Error::Invalid)?);
            if !program.is_absolute() || !directory.is_absolute() {
                return Err(Error::Invalid);
            }
            Transport::Stdio {
                program,
                arguments,
                directory,
            }
        }
        "streamable_http" => {
            if !arguments.is_empty()
                || server.program.is_some()
                || server.directory.is_some()
                || !environment.is_empty()
            {
                return Err(Error::Invalid);
            }
            let url = server.url.ok_or(Error::Invalid)?;
            let parsed = reqwest::Url::parse(&url).map_err(|_| Error::Invalid)?;
            if parsed.scheme() != "https" && parsed.scheme() != "http" {
                return Err(Error::Invalid);
            }
            if !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.fragment().is_some()
            {
                return Err(Error::Invalid);
            }
            if parsed.scheme() == "http" && !is_loopback_host(&parsed) {
                return Err(Error::Invalid);
            }
            validate_text(&url)?;
            Transport::StreamableHttp { url }
        }
        _ => return Err(Error::Invalid),
    };
    Ok(ConfiguredServer {
        server_id,
        environment_id: server.environment_id,
        revision: server.revision,
        transport,
        environment,
        credential_handle: server.credential_handle,
        enabled: server.enabled,
    })
}

fn is_loopback_host(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}

fn validate_text(value: &str) -> Result<(), Error> {
    if value.is_empty() || value.len() > MAX_TEXT_BYTES || value.chars().any(char::is_control) {
        return Err(Error::Invalid);
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};

    #[test]
    fn parses_bounded_host_owned_process_spec() {
        let servers = parse(
            r#"schema_version = 1
            [servers.literature]
            environment_id = "project"
            revision = "sha256:r1"
            program = "/usr/bin/literature-mcp"
            arguments = ["--stdio"]
            directory = "/tmp"
            [servers.literature.environment]
            LANG = "C"
            "#,
        )
        .unwrap();
        assert_eq!(servers[0].server_id, "literature");
        assert_eq!(
            servers[0]
                .process_spec()
                .unwrap()
                .environment
                .get(std::ffi::OsStr::new("LANG"))
                .unwrap(),
            "C"
        );
    }

    #[test]
    fn rejects_relative_programs_unknown_fields_and_inherited_budget() {
        assert_eq!(
            parse(
                "schema_version=1\n[servers.x]\nenvironment_id='p'\nrevision='r'\nprogram='mcp'\ndirectory='/tmp'"
            ),
            Err(Error::Invalid)
        );
        assert_eq!(parse("schema_version=2\n"), Err(Error::Invalid));
        assert_eq!(
            parse("schema_version=1\nunknown=true\n"),
            Err(Error::Invalid)
        );
    }

    #[test]
    fn keeps_credentials_as_opaque_handles() {
        let servers = parse(
            "schema_version=1\n[servers.x]\nenvironment_id='p'\nrevision='r'\nprogram='/usr/bin/x'\ndirectory='/tmp'\ncredential_handle='vault:mcp-x'",
        )
        .unwrap();
        assert_eq!(servers[0].credential_handle.as_deref(), Some("vault:mcp-x"));
        assert!(servers[0].process_spec().unwrap().environment.is_empty());
    }

    #[test]
    fn parses_streamable_http_transport_without_process_fields() {
        let servers = parse(
            "schema_version=1\n[servers.docs]\nenvironment_id='project'\nrevision='r1'\ntransport='streamable_http'\nurl='https://example.test/mcp'",
        )
        .unwrap();
        assert_eq!(
            servers[0].transport,
            Transport::StreamableHttp {
                url: "https://example.test/mcp".into()
            }
        );
        assert!(servers[0].process_spec().is_none());
    }

    #[test]
    fn rejects_plaintext_remote_http_and_url_credentials() {
        assert_eq!(
            parse(
                "schema_version=1\n[servers.docs]\nenvironment_id='project'\nrevision='r1'\ntransport='streamable_http'\nurl='http://example.test/mcp'",
            ),
            Err(Error::Invalid)
        );
        assert_eq!(
            parse(
                "schema_version=1\n[servers.docs]\nenvironment_id='project'\nrevision='r1'\ntransport='streamable_http'\nurl='https://token@example.test/mcp'",
            ),
            Err(Error::Invalid)
        );
        assert!(parse(
            "schema_version=1\n[servers.local]\nenvironment_id='project'\nrevision='r1'\ntransport='streamable_http'\nurl='http://127.0.0.1:7331/mcp'",
        ).is_ok());
    }

    #[test]
    fn reads_only_app_owned_regular_file_without_following_links() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("configuration");
        assert_eq!(read_from_root(&root).unwrap(), None);
        fs::create_dir_all(root.join("agent")).unwrap();
        fs::write(
            root.join("agent/mcp.toml"),
            "schema_version=1\n[servers.x]\nenvironment_id='p'\nrevision='r'\nprogram='/usr/bin/x'\ndirectory='/tmp'",
        )
        .unwrap();
        assert_eq!(read_from_root(&root).unwrap().unwrap().len(), 1);
        fs::remove_file(root.join("agent/mcp.toml")).unwrap();
        symlink("../outside.toml", root.join("agent/mcp.toml")).unwrap();
        assert_eq!(read_from_root(&root), Err(Error::Unavailable));
    }
}
