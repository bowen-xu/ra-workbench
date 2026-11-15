use super::registry;
use crate::agent::instructions::directory::Directory;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::Path,
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    content: String,
    baseline: String,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u8,
    documents: BTreeMap<String, Entry>,
    #[serde(default)]
    receipts: BTreeMap<String, String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: String,
    pub name: String,
    pub workflow: String,
    pub path: String,
    pub content: String,
    pub baseline: String,
    pub revision: String,
}
const FILE: &str = "builtin-documents.v1.json";
const LIMIT: u64 = 2_097_152;
fn error(_: impl std::fmt::Display) -> String {
    "无法访问内置文档存储".into()
}
fn directory(root: &Path) -> Result<Directory, String> {
    // AppStore supplies this trusted root; canonicalize macOS /var and home aliases
    // before the handle-relative, no-follow traversal of all document entries.
    Directory::root(&root.canonicalize().map_err(error)?).map_err(error)
}
fn read_state(root: &Path) -> Result<State, String> {
    if !root.exists() {
        return Ok(State {
            version: 1,
            ..State::default()
        });
    }
    let dir = directory(root)?;
    let Some(file) = dir.config_file(FILE).map_err(error)? else {
        return Ok(State {
            version: 1,
            ..State::default()
        });
    };
    if file.metadata().map_err(error)?.len() > LIMIT {
        return Err("内置文档存储超过容量".into());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    let state: State = serde_json::from_slice(&bytes).map_err(error)?;
    if bytes.len() as u64 > LIMIT
        || state.version != 1
        || state.documents.len() > registry::DEFINITIONS.len()
    {
        return Err("内置文档存储格式无效".into());
    }
    for (id, entry) in &state.documents {
        registry::validate(id, &entry.content)?;
        registry::validate(id, &entry.baseline)?;
    }
    Ok(state)
}
fn installed(root: &Path, path: &str) -> Result<Option<String>, String> {
    if !root.exists() {
        return Ok(None);
    }
    let mut dir = directory(root)?;
    let parts = path.split('/').collect::<Vec<_>>();
    for part in &parts[..parts.len() - 1] {
        let Some(next) = dir.optional_child(Path::new(part)).map_err(error)? else {
            return Ok(None);
        };
        dir = next;
    }
    let Some(file) = dir.config_file(parts[parts.len() - 1]).map_err(error)? else {
        return Ok(None);
    };
    let mut text = String::new();
    file.take(131_073)
        .read_to_string(&mut text)
        .map_err(error)?;
    if text.len() > 131_072 {
        return Err("内置技能超过容量".into());
    }
    Ok(Some(text))
}
fn document(root: &Path, id: &str, state: &State) -> Result<Document, String> {
    let d = registry::definition(id)?;
    let entry = if let Some(entry) = state.documents.get(id) {
        entry.clone()
    } else {
        let fallback = if let Some(path) = d.skill_path {
            installed(root, path)?.unwrap_or_else(|| d.embedded.into())
        } else {
            d.embedded.into()
        };
        Entry {
            content: fallback.clone(),
            baseline: fallback,
        }
    };
    registry::validate(id, &entry.content)?;
    let revision = format!(
        "{:x}",
        Sha256::digest(format!("{}\0{}", entry.content, entry.baseline).as_bytes())
    );
    Ok(Document {
        id: id.into(),
        name: d.name.into(),
        workflow: d.workflow.into(),
        path: super::registry::path(id)?,
        content: entry.content,
        baseline: entry.baseline,
        revision,
    })
}
pub fn read(root: &Path, id: &str) -> Result<Document, String> {
    document(root, id, &read_state(root)?)
}
pub fn list(root: &Path) -> Result<Vec<Document>, String> {
    let state = read_state(root)?;
    registry::DEFINITIONS
        .iter()
        .map(|d| document(root, d.id, &state))
        .collect()
}
fn open_new(dir: &Directory, root: &Path, name: &str) -> Result<File, String> {
    #[cfg(unix)]
    {
        use nix::{
            fcntl::{OFlag, openat},
            sys::stat::Mode,
        };
        let _ = root;
        Ok(File::from(
            openat(
                dir,
                Path::new(name),
                OFlag::O_WRONLY
                    | OFlag::O_CREAT
                    | OFlag::O_EXCL
                    | OFlag::O_NOFOLLOW
                    | OFlag::O_CLOEXEC,
                Mode::from_bits_truncate(0o600),
            )
            .map_err(error)?,
        ))
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(name))
            .map_err(error)
    }
}
fn save(root: &Path, state: &State) -> Result<(), String> {
    let bytes = serde_json::to_vec(state).map_err(error)?;
    if bytes.len() as u64 > LIMIT {
        return Err("内置文档存储超过容量".into());
    }
    let dir = directory(root)?;
    let name = format!(".builtin-{}.tmp", uuid::Uuid::new_v4());
    let mut file = open_new(&dir, root, &name)?;
    file.write_all(&bytes).map_err(error)?;
    file.sync_all().map_err(error)?;
    #[cfg(unix)]
    nix::fcntl::renameat(&dir, Path::new(&name), &dir, Path::new(FILE)).map_err(error)?;
    #[cfg(not(unix))]
    std::fs::rename(root.join(&name), root.join(FILE)).map_err(error)?;
    Ok(())
}
pub fn write(
    root: &Path,
    id: &str,
    expected: &str,
    content: &str,
    baseline: &str,
) -> Result<Document, String> {
    write_inner(root, id, expected, content, baseline, None)
}
pub fn commit(
    root: &Path,
    id: &str,
    expected: &str,
    content: &str,
    baseline: &str,
    request: &str,
) -> Result<Document, String> {
    write_inner(root, id, expected, content, baseline, Some(request))
}
pub fn receipt(root: &Path, request: &str) -> Result<Option<Document>, String> {
    let state = read_state(root)?;
    state
        .receipts
        .get(request)
        .map(|id| document(root, id, &state))
        .transpose()
}
fn write_inner(
    root: &Path,
    id: &str,
    expected: &str,
    content: &str,
    baseline: &str,
    request: Option<&str>,
) -> Result<Document, String> {
    registry::validate(id, content)?;
    registry::validate(id, baseline)?;
    std::fs::create_dir_all(root).map_err(error)?;
    let dir = directory(root)?;
    let lock_name = ".builtin-documents.lock";
    #[cfg(unix)]
    let lock = {
        use nix::{
            fcntl::{OFlag, openat},
            sys::stat::Mode,
        };
        File::from(
            openat(
                &dir,
                Path::new(lock_name),
                OFlag::O_RDWR
                    | OFlag::O_CREAT
                    | OFlag::O_NOFOLLOW
                    | OFlag::O_CLOEXEC
                    | OFlag::O_NONBLOCK,
                Mode::from_bits_truncate(0o600),
            )
            .map_err(error)?,
        )
    };
    #[cfg(not(unix))]
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(lock_name))
        .map_err(error)?;
    if !lock.metadata().map_err(error)?.is_file() {
        return Err("内置文档锁文件无效".into());
    }
    fs2::FileExt::lock_exclusive(&lock).map_err(error)?;
    let mut state = read_state(root)?;
    if let Some(previous) = request.and_then(|r| state.receipts.get(r)) {
        if previous != id {
            return Err("写入请求身份冲突".into());
        }
        return document(root, id, &state);
    }
    let current = document(root, id, &state)?;
    if current.revision != expected {
        return Err("文档已变化，请重新读取；本地编辑已保留".into());
    }
    state.documents.insert(
        id.into(),
        Entry {
            content: content.into(),
            baseline: baseline.into(),
        },
    );
    if let Some(request) = request {
        if state.receipts.len() >= 4096 {
            return Err("内置文档写入回执超过容量".into());
        }
        state.receipts.insert(request.into(), id.into());
    }
    save(root, &state)?;
    document(root, id, &state)
}
/// Overrides affect only the closed application catalog, never workspace skills.
pub fn skill_override(root: &Path, source: &str) -> Result<Option<String>, String> {
    let Some(d) = registry::DEFINITIONS
        .iter()
        .find(|d| d.skill_path == Some(source))
    else {
        return Ok(None);
    };
    Ok(read_state(root)?
        .documents
        .get(d.id)
        .map(|e| e.content.clone()))
}
pub fn prompt(root: &Path, id: &str) -> Result<String, String> {
    Ok(read(root, id)?.content)
}
