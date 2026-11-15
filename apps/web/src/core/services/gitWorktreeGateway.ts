import type { ManuscriptGitWorktree, GitWorktreeGateway } from "@research-assistant/data/writing";
export type { ManuscriptGitWorktree, GitWorktreeGateway } from "@research-assistant/data/writing";
import { resolveAppGateway } from "../runtime/appRuntime";
import { invoke, isDesktopRuntime } from "../runtime/platformCore";

const unavailable = (): never => { throw new Error("Git 隔离工作区只在桌面应用中可用"); };

const browserGateway: GitWorktreeGateway = {
  async list() { return unavailable(); },
  async create() { return unavailable(); },
  async remove() { return unavailable(); },
};

const tauriGateway: GitWorktreeGateway = {
  list: (manuscriptId) => invoke<readonly ManuscriptGitWorktree[]>("agent_list_git_worktrees", { request: { manuscriptId } }),
  create: (manuscriptId, revision) => invoke<ManuscriptGitWorktree>("agent_create_git_worktree", { request: { manuscriptId, ...(revision === undefined ? {} : { revision }) } }),
  remove: async (manuscriptId, worktreeId, discardChanges) => { await invoke("agent_remove_git_worktree", { request: { manuscriptId, worktreeId, discardChanges } }); },
};

export function getGitWorktreeGateway(): GitWorktreeGateway { return resolveAppGateway("getGitWorktreeGateway", isDesktopRuntime() ? tauriGateway : browserGateway); }
