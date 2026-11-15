//! macOS offline file sandbox adapted from Codex sandboxing/src/seatbelt.rs at
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b. Copyright 2025 OpenAI. Apache-2.0;
//! see packages/agent-core/third-party/codex/{LICENSE,NOTICE}.
//! Changes: explicit canonical grants and protected roots, no broad platform
//! defaults or network fallback. Host authorization is still a separate gate.
use super::{ProcessError, ProcessSpec};
use crate::agent::instructions::directory::Directory;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

const EXECUTABLE: &str = "/usr/bin/sandbox-exec";
const BASE: &str = include_str!("seatbelt.sbpl");
const RUNTIME_ROOTS: &[&str] = &[
    "/bin",
    "/sbin",
    "/usr/bin",
    "/usr/sbin",
    "/usr/lib",
    "/usr/libexec",
    "/System/Library",
];
const RUNTIME_FILES: &[&str] = &["/private/var/select/sh", "/private/etc/ssl/openssl.cnf"];

/// Paths come from trusted native services, never deserialized model arguments.
/// Protected roots include application storage, authentication/control data and
/// original research resources that must not be exposed through a parent grant.
pub struct OfflineSandboxPaths {
    pub work: PathBuf,
    pub read_only: Vec<PathBuf>,
    pub protected: Vec<PathBuf>,
    /// Trusted language runtimes may query host limits through sysconf before
    /// opening files. This grants metadata only and remains off for shells.
    pub allow_system_metadata: bool,
}
fn canonical(path: &Path) -> Result<(), ProcessError> {
    if !path.is_absolute() || path.canonicalize()? != path {
        return Err(ProcessError::Invalid);
    }
    Directory::root(path).map_err(|_| ProcessError::Invalid)?;
    Ok(())
}
fn overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
fn validate_protected(path: &Path) -> Result<(), ProcessError> {
    if !path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(ProcessError::Invalid);
    }
    // A reserved file need not exist yet. Verify the nearest existing ancestor
    // rather than allowing a /tmp or symlink alias to evade overlap checks.
    for ancestor in path.ancestors() {
        match ancestor.canonicalize() {
            Ok(actual) if actual == ancestor => return Ok(()),
            Ok(_) => return Err(ProcessError::Invalid),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(ProcessError::Invalid)
}

/// Build a command that can only run through the OS sandbox. Failure of the
/// sandbox executable or profile must be surfaced, never retried unsandboxed.
/// Must run on a blocking worker; caller revalidates lease and grants at spawn.
pub fn prepare_offline_sandbox(
    mut command: ProcessSpec,
    paths: OfflineSandboxPaths,
) -> Result<ProcessSpec, ProcessError> {
    command.validate()?;
    if paths.read_only.len() > 32 || paths.protected.is_empty() || paths.protected.len() > 64 {
        return Err(ProcessError::Invalid);
    }
    canonical(&paths.work)?;
    if command.directory != paths.work {
        return Err(ProcessError::Invalid);
    }
    for path in &paths.read_only {
        canonical(path)?;
    }
    for path in &paths.protected {
        validate_protected(path)?;
        if overlap(path, &paths.work)
            || paths.read_only.iter().any(|read| overlap(path, read))
            || RUNTIME_ROOTS
                .iter()
                .chain(RUNTIME_FILES)
                .any(|read| overlap(path, Path::new(read)))
        {
            return Err(ProcessError::Invalid);
        }
    }
    // Arbitrary environment injection belongs neither to path grants nor to
    // model args. The host supplies only this small runtime allowlist here.
    const ENV: &[&str] = &["PATH", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "NO_COLOR"];
    if command
        .environment
        .keys()
        .any(|key| !ENV.iter().any(|name| key == name))
    {
        return Err(ProcessError::Invalid);
    }
    command
        .environment
        .insert("HOME".into(), paths.work.clone().into_os_string());
    command
        .environment
        .insert("TMPDIR".into(), paths.work.clone().into_os_string());
    let mut profile = BASE.to_owned();
    if paths.allow_system_metadata {
        profile.push_str("\n(allow sysctl-read)");
    }
    let mut arguments: Vec<OsString> = vec!["-D".into()];
    let mut work = OsString::from("WORK=");
    work.push(&paths.work);
    arguments.push(work);
    for (index, path) in paths.read_only.iter().enumerate() {
        profile.push_str(&format!(
            "\n(allow file-read* file-map-executable (subpath (param \"READ_{index}\")))"
        ));
        arguments.push("-D".into());
        let mut value = OsString::from(format!("READ_{index}="));
        value.push(path);
        arguments.push(value);
    }
    // Explicit denies also protect against runtime aliases/symlinks resolving
    // into a reserved root. Params avoid interpreting path text as SBPL code.
    for (index, path) in paths.protected.iter().enumerate() {
        profile.push_str(&format!("\n(deny file-read* file-write* file-map-executable (subpath (param \"PROTECTED_{index}\")))"));
        arguments.push("-D".into());
        let mut value = OsString::from(format!("PROTECTED_{index}="));
        value.push(path);
        arguments.push(value);
    }
    arguments.extend([
        "-p".into(),
        profile.into(),
        "--".into(),
        command.program.into_os_string(),
    ]);
    arguments.extend(command.arguments);
    Ok(ProcessSpec {
        program: EXECUTABLE.into(),
        arguments,
        ..command
    })
}

#[cfg(test)]
#[path = "seatbelt/tests.rs"]
mod tests;
