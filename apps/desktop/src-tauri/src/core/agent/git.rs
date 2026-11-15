//! Native Git/worktree adapter.
//!
//! Command construction and repository discovery are adapted from Codex
//! `git-utils` (Apache-2.0, fixed ref recorded in the migration map). The
//! domain types and porcelain parser live in `research-agent-core`; this
//! module is the only process/filesystem boundary.

use research_agent_core::git::{
    GitCommitSummary, GitError, GitRepository, GitStatusEntry, GitWorktree, parse_log_records,
    parse_remote_urls, parse_status_porcelain_v2, parse_worktree_porcelain, validate_branch_name,
};
use research_agent_core::workspace::{FileChange, FileChangeKind, FileVersion, WorkspaceFile};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::{
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};
use tokio::{process::Command, time::timeout};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_DIFF_BYTES: usize = 16 * 1024 * 1024;
const MAX_SNAPSHOT_FILES: usize = 10_000;
const MAX_SNAPSHOT_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum GitServiceError {
    #[error(transparent)]
    Core(#[from] GitError),
    #[error("git path is invalid or outside an authorized directory")]
    Path,
    #[error("git command failed: {0}")]
    Command(String),
    #[error("git command timed out")]
    Timeout,
    #[error("git output is not valid UTF-8")]
    Utf8,
}

/// Immutable file bytes from a validated Git revision. This is deliberately a
/// native adapter type: callers must convert it into the application's
/// FileVersion/ChangeSet contracts before any write is possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRevisionFile {
    pub path: String,
    pub bytes: Vec<u8>,
    pub executable: bool,
}

/// Convert an immutable Git tree into the application's change inventory. The
/// result intentionally carries no `SourceVersion`: the writing adapter must
/// attach the current editor/disk identity before publishing a ChangeSet.
pub fn revision_changes(
    baseline: &[WorkspaceFile],
    revision: &[GitRevisionFile],
) -> Result<Vec<FileChange>, GitServiceError> {
    if baseline.len() > MAX_SNAPSHOT_FILES || revision.len() > MAX_SNAPSHOT_FILES {
        return Err(GitServiceError::Path);
    }
    let mut before = BTreeMap::new();
    for file in baseline {
        research_agent_core::workspace::validate_path(&file.path)
            .map_err(|_| GitServiceError::Path)?;
        if before
            .insert(file.path.clone(), file.version.clone())
            .is_some()
        {
            return Err(GitServiceError::Path);
        }
    }
    let mut after = BTreeMap::new();
    let mut total = 0usize;
    for file in revision {
        research_agent_core::workspace::validate_path(&file.path)
            .map_err(|_| GitServiceError::Path)?;
        total = total
            .checked_add(file.bytes.len())
            .filter(|size| *size <= MAX_SNAPSHOT_BYTES)
            .ok_or(GitServiceError::Path)?;
        let version = FileVersion {
            sha256: format!("{:x}", Sha256::digest(&file.bytes)),
            byte_size: file.bytes.len() as u64,
            executable: file.executable,
        };
        if after.insert(file.path.clone(), version).is_some() {
            return Err(GitServiceError::Path);
        }
    }
    let mut paths = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    Ok(paths
        .into_iter()
        .filter_map(|path| {
            let old = before.get(&path);
            let new = after.get(&path);
            if old == new {
                return None;
            }
            let kind = match (old, new) {
                (None, Some(_)) => FileChangeKind::Added,
                (Some(_), None) => FileChangeKind::Deleted,
                (Some(_), Some(_)) => FileChangeKind::Modified,
                (None, None) => unreachable!(),
            };
            Some(FileChange {
                kind,
                path,
                before: old.cloned(),
                after: new.cloned(),
                source: None,
            })
        })
        .collect())
}

#[derive(Clone, Copy, Debug)]
pub struct GitWorkspaceService {
    timeout: Duration,
}

impl Default for GitWorkspaceService {
    fn default() -> Self {
        Self {
            timeout: COMMAND_TIMEOUT,
        }
    }
}

impl GitWorkspaceService {
    pub fn new(timeout: Duration) -> Result<Self, GitServiceError> {
        if !(Duration::from_secs(1)..=Duration::from_secs(60)).contains(&timeout) {
            return Err(GitServiceError::Path);
        }
        Ok(Self { timeout })
    }

    pub async fn inspect(&self, path: &Path) -> Result<GitRepository, GitServiceError> {
        let cwd = canonical_directory(path)?;
        let root = self.stdout(&cwd, &["rev-parse", "--show-toplevel"]).await?;
        let git_dir = self.stdout(&cwd, &["rev-parse", "--git-dir"]).await?;
        let root = canonical_directory(Path::new(&root))?;
        let git_dir = if Path::new(&git_dir).is_absolute() {
            PathBuf::from(git_dir)
        } else {
            root.join(git_dir)
        };
        let head = self
            .stdout(&root, &["rev-parse", "--verify", "HEAD"])
            .await
            .ok()
            .and_then(|value| research_agent_core::git::GitSha::new(&value).ok());
        let branch = self
            .stdout(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .await
            .ok()
            .filter(|value| !value.is_empty());
        let remote_output = self
            .stdout(&root, &["remote", "-v"])
            .await
            .unwrap_or_default();
        let status = self
            .output(&root, &["status", "--porcelain=v2", "-z", "--branch"])
            .await?;
        let (_, _) = parse_status_porcelain_v2(&status.stdout)?;
        Ok(GitRepository {
            root: root.to_string_lossy().into_owned(),
            git_dir: git_dir.to_string_lossy().into_owned(),
            worktree: Some(root.to_string_lossy().into_owned()),
            head,
            branch,
            remotes: parse_remote_urls(&remote_output),
        })
    }

    /// Initialize version metadata only after an explicit user action. It does
    /// not stage files, create a commit, or alter the manuscript contents.
    pub async fn initialize(&self, path: &Path) -> Result<GitRepository, GitServiceError> {
        let cwd = canonical_directory(path)?;
        if let Ok(repository) = self.inspect(&cwd).await {
            return Ok(repository);
        }
        self.run(&cwd, &["init", "--initial-branch=main"]).await?;
        self.inspect(&cwd).await
    }

    pub async fn status(
        &self,
        repository: &GitRepository,
    ) -> Result<(Option<String>, Vec<GitStatusEntry>), GitServiceError> {
        let root = authorized_root(repository)?;
        let output = self
            .output(&root, &["status", "--porcelain=v2", "-z", "--branch"])
            .await?;
        Ok(parse_status_porcelain_v2(&output.stdout)?)
    }

    pub async fn list_worktrees(
        &self,
        repository: &GitRepository,
    ) -> Result<Vec<GitWorktree>, GitServiceError> {
        let root = authorized_root(repository)?;
        let output = self
            .stdout(&root, &["worktree", "list", "--porcelain"])
            .await?;
        let mut worktrees = parse_worktree_porcelain(&output)?;
        for worktree in &mut worktrees {
            let canonical = canonical_directory(Path::new(&worktree.path))?;
            worktree.path = canonical.to_string_lossy().into_owned();
        }
        Ok(worktrees)
    }

    /// Reopen a known worktree only when Git inventory and canonical path agree.
    pub async fn open_worktree(
        &self,
        repository: &GitRepository,
        target: &Path,
    ) -> Result<GitWorktree, GitServiceError> {
        let target = canonical_directory(target)?;
        self.list_worktrees(repository)
            .await?
            .into_iter()
            .find(|worktree| Path::new(&worktree.path) == target)
            .ok_or(GitServiceError::Path)
    }

    /// Adapted from Codex `git-utils/src/branch.rs`: merge-base is read-only
    /// and remains independent from the worktree lifecycle.
    pub async fn merge_base(
        &self,
        repository: &GitRepository,
        branch: &str,
    ) -> Result<Option<String>, GitServiceError> {
        validate_branch_name(branch)?;
        let root = authorized_root(repository)?;
        let head = match self.stdout(&root, &["rev-parse", "--verify", "HEAD"]).await {
            Ok(head) => head,
            Err(GitServiceError::Command(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        let branch_ref = match self.stdout(&root, &["rev-parse", "--verify", branch]).await {
            Ok(reference) => reference,
            Err(GitServiceError::Command(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(Some(
            self.stdout(&root, &["merge-base", &head, &branch_ref])
                .await?,
        ))
    }

    pub async fn create_branch(
        &self,
        repository: &GitRepository,
        branch: &str,
        revision: &str,
    ) -> Result<(), GitServiceError> {
        validate_branch_name(branch)?;
        if revision.trim().is_empty()
            || revision.len() > 256
            || revision.starts_with('-')
            || revision.chars().any(char::is_control)
        {
            return Err(GitServiceError::Path);
        }
        let root = authorized_root(repository)?;
        self.run(&root, &["branch", "--", branch, revision]).await
    }

    pub async fn delete_branch(
        &self,
        repository: &GitRepository,
        branch: &str,
    ) -> Result<(), GitServiceError> {
        validate_branch_name(branch)?;
        if repository.branch.as_deref() == Some(branch) {
            return Err(GitServiceError::Path);
        }
        let root = authorized_root(repository)?;
        self.run(&root, &["branch", "-d", "--", branch]).await
    }

    pub async fn list_branches(
        &self,
        repository: &GitRepository,
    ) -> Result<Vec<String>, GitServiceError> {
        let root = authorized_root(repository)?;
        let output = self
            .stdout(
                &root,
                &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
            )
            .await?;
        let mut branches = Vec::new();
        for branch in output
            .lines()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            validate_branch_name(branch)?;
            branches.push(branch.to_owned());
            if branches.len() > 1000 {
                return Err(GitServiceError::Path);
            }
        }
        Ok(branches)
    }

    pub async fn log(
        &self,
        repository: &GitRepository,
        limit: usize,
    ) -> Result<Vec<GitCommitSummary>, GitServiceError> {
        if !(1..=200).contains(&limit) {
            return Err(GitServiceError::Path);
        }
        let root = authorized_root(repository)?;
        let limit = limit.to_string();
        let output = self
            .output(
                &root,
                &[
                    "log",
                    "--no-decorate",
                    "--format=%H%x00%ct%x00%s%x00",
                    "-n",
                    &limit,
                ],
            )
            .await?;
        parse_log_records(&output.stdout).map_err(Into::into)
    }

    pub async fn diff(&self, repository: &GitRepository) -> Result<Vec<u8>, GitServiceError> {
        let root = authorized_root(repository)?;
        let output = self
            .output(
                &root,
                &["diff", "--no-ext-diff", "--no-color", "--binary", "--"],
            )
            .await?;
        if output.stdout.len() > MAX_DIFF_BYTES {
            return Err(GitServiceError::Path);
        }
        Ok(output.stdout)
    }

    pub async fn diff_revisions(
        &self,
        repository: &GitRepository,
        from: &str,
        to: &str,
    ) -> Result<Vec<u8>, GitServiceError> {
        for revision in [from, to] {
            if revision.trim().is_empty()
                || revision.len() > 256
                || revision.starts_with('-')
                || revision.chars().any(char::is_control)
            {
                return Err(GitServiceError::Path);
            }
        }
        let root = authorized_root(repository)?;
        let output = self
            .output(
                &root,
                &[
                    "diff",
                    "--no-ext-diff",
                    "--no-color",
                    "--binary",
                    from,
                    to,
                    "--",
                ],
            )
            .await?;
        if output.stdout.len() > MAX_DIFF_BYTES {
            return Err(GitServiceError::Path);
        }
        Ok(output.stdout)
    }

    /// Create a manuscript version from the user's already staged snapshot.
    ///
    /// We intentionally do not run `git add` here: staging is a user-visible
    /// boundary and changing it would silently absorb dirty editor content or
    /// unrelated files. A version can only be created when the worktree has
    /// no unstaged or untracked entries.
    pub async fn commit_staged(
        &self,
        repository: &GitRepository,
        message: &str,
    ) -> Result<GitCommitSummary, GitServiceError> {
        let message = message.trim();
        if message.is_empty()
            || message.len() > 4096
            || message.chars().any(|character| character == '\0')
        {
            return Err(GitServiceError::Path);
        }
        let (branch, entries) = self.status(repository).await?;
        if branch.is_none() || entries.iter().any(|entry| entry.unstaged) {
            return Err(GitServiceError::Path);
        }
        if entries.is_empty() {
            return Err(GitServiceError::Path);
        }
        let root = authorized_root(repository)?;
        self.run(&root, &["commit", "--no-verify", "--message", message])
            .await?;
        self.log(repository, 1)
            .await?
            .into_iter()
            .next()
            .ok_or(GitServiceError::Command("commit produced no HEAD".into()))
    }

    /// Read one bounded file from an immutable Git revision. This only feeds
    /// historical previews; callers still need a ChangeSet before applying it.
    pub async fn show_file(
        &self,
        repository: &GitRepository,
        revision: &str,
        relative_path: &str,
    ) -> Result<Vec<u8>, GitServiceError> {
        let revision = research_agent_core::git::GitSha::new(revision)?;
        research_agent_core::workspace::validate_path(relative_path)
            .map_err(|_| GitServiceError::Path)?;
        if relative_path.len() > 4096 {
            return Err(GitServiceError::Path);
        }
        let spec = format!("{}:{relative_path}", revision.as_str());
        let root = authorized_root(repository)?;
        let output = self
            .output(&root, &["show", "--no-ext-diff", &spec])
            .await?;
        if output.stdout.len() > 8 * 1024 * 1024 {
            return Err(GitServiceError::Path);
        }
        Ok(output.stdout)
    }

    /// Read a bounded, complete tree from an immutable revision. The command
    /// shape follows Codex git-utils' repository snapshot boundary, while the
    /// byte and path limits are owned by this app. No checkout or worktree is
    /// created, so this cannot disturb a dirty manuscript.
    pub async fn snapshot_revision(
        &self,
        repository: &GitRepository,
        revision: &str,
    ) -> Result<Vec<GitRevisionFile>, GitServiceError> {
        let revision = research_agent_core::git::GitSha::new(revision)?;
        let root = authorized_root(repository)?;
        let output = self
            .stdout_bytes(
                &root,
                &[
                    "ls-tree",
                    "-r",
                    "-z",
                    "--full-tree",
                    "--name-only",
                    revision.as_str(),
                ],
            )
            .await?;
        let mut files = Vec::new();
        let mut total = 0usize;
        for raw in output
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
        {
            let path = std::str::from_utf8(raw).map_err(|_| GitServiceError::Utf8)?;
            research_agent_core::workspace::validate_path(path)
                .map_err(|_| GitServiceError::Path)?;
            if files.len() >= MAX_SNAPSHOT_FILES {
                return Err(GitServiceError::Path);
            }
            let bytes = self.show_file(repository, revision.as_str(), path).await?;
            total = total
                .checked_add(bytes.len())
                .filter(|size| *size <= MAX_SNAPSHOT_BYTES)
                .ok_or(GitServiceError::Path)?;
            let executable = self
                .stdout(&root, &["ls-tree", revision.as_str(), "--", path])
                .await
                .ok()
                .and_then(|line| line.split_whitespace().next().map(|mode| mode == "100755"))
                .unwrap_or(false);
            files.push(GitRevisionFile {
                path: path.to_owned(),
                bytes,
                executable,
            });
        }
        Ok(files)
    }

    /// Create an application-owned detached worktree. Branches and revisions
    /// are passed as positional Git values after validation; no shell is used.
    pub async fn create_worktree(
        &self,
        repository: &GitRepository,
        target: &Path,
        revision: &str,
    ) -> Result<PathBuf, GitServiceError> {
        let root = authorized_root(repository)?;
        let target = new_child_path(target)?;
        if revision.trim().is_empty()
            || revision.len() > 256
            || revision.starts_with('-')
            || revision.chars().any(char::is_control)
        {
            return Err(GitServiceError::Path);
        }
        self.run(
            &root,
            &[
                "worktree",
                "add",
                "--detach",
                target.to_string_lossy().as_ref(),
                revision,
            ],
        )
        .await?;
        Ok(target)
    }

    /// Remove only an explicitly supplied worktree path after Git verifies it
    /// belongs to this repository. The caller still owns higher-level lease and
    /// dirty-change confirmation; `force` is never inferred from status.
    pub async fn remove_worktree(
        &self,
        repository: &GitRepository,
        target: &Path,
        force: bool,
    ) -> Result<(), GitServiceError> {
        let root = authorized_root(repository)?;
        let target = canonical_directory(target)?;
        let listed = self.list_worktrees(repository).await?;
        if !listed
            .iter()
            .any(|worktree| Path::new(&worktree.path) == target)
        {
            return Err(GitServiceError::Path);
        }
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        let target_arg = target.to_string_lossy().into_owned();
        args.push(&target_arg);
        self.run(&root, &args).await
    }

    async fn stdout(&self, cwd: &Path, args: &[&str]) -> Result<String, GitServiceError> {
        let output = self.output(cwd, args).await?;
        String::from_utf8(output.stdout)
            .map(|text| text.trim().to_owned())
            .map_err(|_| GitServiceError::Utf8)
    }
    async fn stdout_bytes(&self, cwd: &Path, args: &[&str]) -> Result<Vec<u8>, GitServiceError> {
        Ok(self.output(cwd, args).await?.stdout)
    }
    async fn run(&self, cwd: &Path, args: &[&str]) -> Result<(), GitServiceError> {
        self.output(cwd, args).await.map(|_| ())
    }
    async fn output(&self, cwd: &Path, args: &[&str]) -> Result<Output, GitServiceError> {
        let mut command = Command::new("git");
        command
            .current_dir(cwd)
            .arg("-c")
            .arg(if cfg!(windows) {
                "core.hooksPath=NUL"
            } else {
                "core.hooksPath=/dev/null"
            })
            .args(args);
        let output = timeout(self.timeout, command.output())
            .await
            .map_err(|_| GitServiceError::Timeout)?
            .map_err(|error| GitServiceError::Command(error.to_string()))?;
        if !output.status.success() {
            return Err(GitServiceError::Command(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        Ok(output)
    }
}

fn canonical_directory(path: &Path) -> Result<PathBuf, GitServiceError> {
    if !path.is_absolute() {
        return Err(GitServiceError::Path);
    }
    let path = path.canonicalize().map_err(|_| GitServiceError::Path)?;
    if !path.is_dir() {
        return Err(GitServiceError::Path);
    }
    Ok(path)
}
fn new_child_path(path: &Path) -> Result<PathBuf, GitServiceError> {
    if !path.is_absolute() || path.exists() {
        return Err(GitServiceError::Path);
    }
    let parent = canonical_directory(path.parent().ok_or(GitServiceError::Path)?)?;
    Ok(parent.join(path.file_name().ok_or(GitServiceError::Path)?))
}
fn authorized_root(repository: &GitRepository) -> Result<PathBuf, GitServiceError> {
    let root = canonical_directory(Path::new(&repository.root))?;
    if repository.worktree.as_deref() != Some(root.to_string_lossy().as_ref()) {
        return Err(GitServiceError::Path);
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;
    use tempfile::tempdir;

    fn git(dir: &Path, args: &[&str]) {
        assert!(
            StdCommand::new("git")
                .current_dir(dir)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    fn repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        git(dir.path(), &["init", "--initial-branch=main"]);
        git(dir.path(), &["config", "user.name", "Test"]);
        git(dir.path(), &["config", "user.email", "test@example.com"]);
        std::fs::write(dir.path().join("main.tex"), "x").unwrap();
        git(dir.path(), &["add", "main.tex"]);
        git(dir.path(), &["commit", "-m", "initial"]);
        dir
    }

    #[tokio::test]
    async fn initializes_only_after_explicit_call_and_is_idempotent() {
        let source = tempdir().unwrap();
        std::fs::write(source.path().join("main.tex"), "draft").unwrap();
        let service = GitWorkspaceService::default();
        assert!(service.inspect(source.path()).await.is_err());
        let initialized = service.initialize(source.path()).await.unwrap();
        assert_eq!(initialized.branch.as_deref(), Some("main"));
        assert!(initialized.head.is_none());
        assert!(source.path().join("main.tex").exists());
        let second = service.initialize(source.path()).await.unwrap();
        assert_eq!(second.root, initialized.root);
        assert!(second.head.is_none());
    }

    #[tokio::test]
    async fn inspects_status_and_owns_worktree_lifecycle() {
        let source = repo();
        let service = GitWorkspaceService::default();
        let info = service.inspect(source.path()).await.unwrap();
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert_eq!(service.log(&info, 1).await.unwrap().len(), 1);
        assert_eq!(
            service.merge_base(&info, "main").await.unwrap(),
            info.head.as_ref().map(|head| head.as_str().to_owned())
        );
        service
            .create_branch(&info, "agent/review", "HEAD")
            .await
            .unwrap();
        assert_eq!(
            service.list_branches(&info).await.unwrap(),
            vec!["agent/review", "main"]
        );
        std::fs::write(source.path().join("main.tex"), "changed").unwrap();
        assert_eq!(
            service.status(&info).await.unwrap().1[0].kind,
            research_agent_core::git::GitChangeKind::Modified
        );
        let target = source.path().parent().unwrap().join("agent-worktree");
        service
            .create_worktree(&info, &target, "HEAD")
            .await
            .unwrap();
        assert!(target.join(".git").exists());
        assert_eq!(std::fs::read(target.join("main.tex")).unwrap(), b"x");
        let opened = service.open_worktree(&info, &target).await.unwrap();
        assert_eq!(
            opened.path,
            target.canonicalize().unwrap().to_string_lossy()
        );
        assert!(
            service
                .status(&GitRepository {
                    root: opened.path.clone(),
                    git_dir: info.git_dir.clone(),
                    worktree: Some(opened.path.clone()),
                    head: opened.head.clone(),
                    branch: opened.branch.clone(),
                    remotes: Default::default(),
                })
                .await
                .unwrap()
                .1
                .is_empty()
        );
        service.remove_worktree(&info, &target, true).await.unwrap();
        assert!(!target.exists());
        service.delete_branch(&info, "agent/review").await.unwrap();
    }

    #[tokio::test]
    async fn commits_only_an_already_staged_clean_snapshot() {
        let source = repo();
        let service = GitWorkspaceService::default();
        let info = service.inspect(source.path()).await.unwrap();
        std::fs::write(source.path().join("main.tex"), "staged").unwrap();
        git(source.path(), &["add", "main.tex"]);
        let version = service
            .commit_staged(&info, "save staged manuscript")
            .await
            .unwrap();
        assert_eq!(version.subject, "save staged manuscript");
        assert!(service.status(&info).await.unwrap().1.is_empty());

        std::fs::write(source.path().join("main.tex"), "dirty").unwrap();
        let info = service.inspect(source.path()).await.unwrap();
        assert!(
            service
                .commit_staged(&info, "must reject dirty tree")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn diffs_two_immutable_revisions_with_a_bounded_result() {
        let source = repo();
        let service = GitWorkspaceService::default();
        let info = service.inspect(source.path()).await.unwrap();
        std::fs::write(source.path().join("main.tex"), "second\n").unwrap();
        git(source.path(), &["add", "main.tex"]);
        git(source.path(), &["commit", "-m", "second"]);
        let commits = service.log(&info, 2).await.unwrap();
        let diff = service
            .diff_revisions(&info, commits[1].sha.as_str(), commits[0].sha.as_str())
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&diff).contains("-x"));
        assert!(String::from_utf8_lossy(&diff).contains("+second"));
    }

    #[tokio::test]
    async fn reads_only_a_bounded_file_from_a_commit() {
        let source = repo();
        let service = GitWorkspaceService::default();
        let info = service.inspect(source.path()).await.unwrap();
        let head = info.head.clone().unwrap();
        assert_eq!(
            service
                .show_file(&info, head.as_str(), "main.tex")
                .await
                .unwrap(),
            b"x"
        );
        assert!(
            service
                .show_file(&info, head.as_str(), "../secret")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn snapshots_an_immutable_revision_without_touching_dirty_files() {
        let source = repo();
        let service = GitWorkspaceService::default();
        let info = service.inspect(source.path()).await.unwrap();
        let head = info.head.clone().unwrap();
        std::fs::write(source.path().join("main.tex"), "dirty editor buffer").unwrap();
        let files = service
            .snapshot_revision(&info, head.as_str())
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "main.tex");
        assert_eq!(files[0].bytes, b"x");
        assert!(!files[0].executable);
        assert_eq!(
            std::fs::read(source.path().join("main.tex")).unwrap(),
            b"dirty editor buffer"
        );
    }

    #[test]
    fn converts_revision_bytes_to_a_change_inventory_without_source_authority() {
        let baseline = vec![WorkspaceFile {
            path: "main.tex".into(),
            version: FileVersion {
                sha256: format!("{:x}", Sha256::digest(b"old")),
                byte_size: 3,
                executable: false,
            },
        }];
        let revision = vec![
            GitRevisionFile {
                path: "main.tex".into(),
                bytes: b"new".to_vec(),
                executable: false,
            },
            GitRevisionFile {
                path: "sections/intro.tex".into(),
                bytes: b"intro".to_vec(),
                executable: false,
            },
        ];
        let changes = revision_changes(&baseline, &revision).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].kind, FileChangeKind::Modified);
        assert_eq!(changes[1].kind, FileChangeKind::Added);
        assert!(changes.iter().all(|change| change.source.is_none()));
    }
}
