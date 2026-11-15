import type { ManuscriptVersionCommit, ManuscriptVersionHistory, ManuscriptVersionGateway, GitRestoreScope, ManuscriptGitRestoreProposal, GitRepositoryView, ManuscriptGitBranches, ManuscriptVersionDiff, HistoricalVersionFile } from "@research-assistant/data/writing";
export type { ManuscriptVersionCommit, ManuscriptVersionHistory, ManuscriptVersionGateway, GitRestoreScope, ManuscriptGitRestoreProposal, GitRepositoryView, ManuscriptGitBranches, ManuscriptVersionDiff, HistoricalVersionFile } from "@research-assistant/data/writing";
import { resolveAppGateway } from "../runtime/appRuntime";
import { invoke } from "../runtime/platformCore";
import { isDesktopRuntime } from "../runtime/platformCore";

const unavailable = (): never => {
  throw new Error("Git 版本只在桌面应用中可用");
};

const browserGitVersionGateway: ManuscriptVersionGateway = {
  async initialize() { return unavailable(); },
  async listBranches() { return unavailable(); },
  async createBranch() { return unavailable(); },
  async deleteBranch() { return unavailable(); },
  async diffVersions() { return unavailable(); },
  async list() { return unavailable(); },
  async create() { return unavailable(); },
  async readFile() { return unavailable(); },
  async restoreVersion() { return unavailable(); },
  async latestRestoreScope() { return unavailable(); },
};

const tauriGitVersionGateway: ManuscriptVersionGateway = {
  async initialize(manuscriptId) {
    return invoke<GitRepositoryView>("agent_initialize_manuscript_git_repository", { request: { manuscriptId } });
  },
  async listBranches(manuscriptId) {
    return invoke<ManuscriptGitBranches>("agent_list_manuscript_git_branches", { manuscriptId });
  },
  async createBranch(manuscriptId, branch) {
    return invoke<ManuscriptGitBranches>("agent_create_manuscript_git_branch", { request: { manuscriptId, branch } });
  },
  async deleteBranch(manuscriptId, branch) {
    return invoke<ManuscriptGitBranches>("agent_delete_manuscript_git_branch", { request: { manuscriptId, branch } });
  },
  async diffVersions(manuscriptId, fromRevision, toRevision) {
    return invoke<ManuscriptVersionDiff>("agent_diff_manuscript_git_versions", { request: { manuscriptId, fromRevision, toRevision } });
  },
  async list(manuscriptId, limit) {
    const result = await invoke<ManuscriptVersionHistory>("agent_list_manuscript_git_versions", {
      request: { manuscriptId, ...(limit === undefined ? {} : { limit }) },
    });
    return result;
  },
  async create(manuscriptId, message) {
    const result = await invoke<{ commit: ManuscriptVersionCommit }>("agent_create_manuscript_git_version", {
      request: { manuscriptId, message },
    });
    return result.commit;
  },
  async readFile(manuscriptId, revision, relativePath) {
    return invoke<HistoricalVersionFile>("agent_read_manuscript_git_version_file", {
      request: { manuscriptId, revision, relativePath },
    });
  },
  async restoreVersion(scope, revision) {
    return invoke<ManuscriptGitRestoreProposal>("agent_restore_manuscript_git_version", {
      request: { ...scope, revision },
    });
  },
  async latestRestoreScope(conversationId, manuscriptId) {
    return invoke<GitRestoreScope | null>("agent_latest_manuscript_git_restore_scope", {
      request: { conversationId, manuscriptId },
    });
  },
};

export function getManuscriptVersionGateway(): ManuscriptVersionGateway {
  return resolveAppGateway("getManuscriptVersionGateway", isDesktopRuntime() ? tauriGitVersionGateway : browserGitVersionGateway);
}
