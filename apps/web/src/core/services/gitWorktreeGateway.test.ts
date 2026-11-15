import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {getGitWorktreeGateway, type GitWorktreeGateway} from "../public/gitWorktrees";
import {selectAppRuntime, type AppRuntime} from "../runtime/appRuntime";

const native = vi.hoisted(() => ({invoke: vi.fn(), isTauri: () => false}));
vi.mock("@tauri-apps/api/core", () => native);

const storage: Storage = {length: 0, key: () => null, getItem: () => null, setItem() {}, removeItem() {}, clear() {}};
const tree = {schemaVersion: 1, id: "worktree", manuscriptId: "manuscript", repositoryRoot: "/source",
  worktreePath: "/worktree", revision: "main", state: "active" as const};
function select(mode: AppRuntime["mode"], gateways: AppRuntime["gateways"] = {}): void {
  selectAppRuntime({mode, storage, gateways, dispose() {}});
}
beforeEach(() => {native.invoke.mockReset(); select("browser");});
afterEach(() => select("browser"));

it("retains the exact three desktop commands, arguments, results and stable port identity", async () => {
  select("desktop");
  const gateway = getGitWorktreeGateway(), rows = [tree];
  expect(getGitWorktreeGateway()).toBe(gateway);
  expect(Object.keys(gateway).sort()).toEqual(["create", "list", "remove"]);
  native.invoke.mockResolvedValueOnce(rows).mockResolvedValueOnce(tree).mockResolvedValueOnce(tree).mockResolvedValueOnce("ignored");
  await expect(gateway.list("manuscript")).resolves.toBe(rows);
  await expect(gateway.create("manuscript")).resolves.toBe(tree);
  await expect(gateway.create("manuscript", "frozen-revision")).resolves.toBe(tree);
  await expect(gateway.remove("manuscript", "worktree", false)).resolves.toBeUndefined();
  expect(native.invoke.mock.calls).toEqual([
    ["agent_list_git_worktrees", {request: {manuscriptId: "manuscript"}}],
    ["agent_create_git_worktree", {request: {manuscriptId: "manuscript"}}],
    ["agent_create_git_worktree", {request: {manuscriptId: "manuscript", revision: "frozen-revision"}}],
    ["agent_remove_git_worktree", {request: {manuscriptId: "manuscript", worktreeId: "worktree", discardChanges: false}}],
  ]);
});

it("preserves a native failure and its diagnostic identity", async () => {
  select("desktop");
  const failure = {code: "dirty_worktree", requestId: "request-7", message: "preserved failure"};
  native.invoke.mockRejectedValueOnce(failure);
  await expect(getGitWorktreeGateway().remove("manuscript", "worktree", true)).rejects.toBe(failure);
  expect(native.invoke).toHaveBeenCalledWith("agent_remove_git_worktree", {
    request: {manuscriptId: "manuscript", worktreeId: "worktree", discardChanges: true},
  });
});

it("retains the browser unavailable behavior without issuing native operations", async () => {
  const gateway = getGitWorktreeGateway();
  for (const operation of [() => gateway.list("manuscript"), () => gateway.create("manuscript"),
    () => gateway.remove("manuscript", "worktree", false)])
    await expect(operation()).rejects.toThrow("Git 隔离工作区只在桌面应用中可用");
  expect(native.invoke).not.toHaveBeenCalled();
});

it("uses the selected preview port by identity and rejects a missing preview override", async () => {
  const override: GitWorktreeGateway = {list: vi.fn(async () => [tree]), create: vi.fn(async () => tree), remove: vi.fn(async () => undefined)};
  select("preview", {getGitWorktreeGateway: override});
  expect(getGitWorktreeGateway()).toBe(override);
  await expect(getGitWorktreeGateway().list("manuscript")).resolves.toEqual([tree]);
  expect(override.list).toHaveBeenCalledWith("manuscript");
  select("preview");
  expect(() => getGitWorktreeGateway()).toThrow("预览未覆盖业务网关：getGitWorktreeGateway");
  expect(native.invoke).not.toHaveBeenCalled();
});
