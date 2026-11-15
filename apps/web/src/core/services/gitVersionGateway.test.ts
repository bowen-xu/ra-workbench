import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {getManuscriptVersionGateway, type ManuscriptVersionGateway} from "../public/gitVersions";
import {selectAppRuntime, type AppRuntime} from "../runtime/appRuntime";

const native = vi.hoisted(() => ({invoke: vi.fn(), isTauri: () => false}));
vi.mock("@tauri-apps/api/core", () => native);
const storage: Storage = {length: 0, key: () => null, getItem: () => null, setItem() {}, removeItem() {}, clear() {}};
const manuscriptId = "manuscript", commit = {sha: "commit-sha", timestampUnixSeconds: 7, subject: "saved version"};
const scope = {conversationId: "conversation", requestId: "request", manuscriptId, contextSnapshotId: "snapshot"};
const repository = {schemaVersion: 1, manuscriptId, root: "/source"};
const branches = {schemaVersion: 1, manuscriptId, current: "main", branches: ["main"]};
const history = {schemaVersion: 1, manuscriptId, commits: [commit]};
const diff = {schemaVersion: 1, manuscriptId, fromRevision: "before", toRevision: "after", kind: "text", content: "patch", byteSize: 5};
const file = {schemaVersion: 1, manuscriptId, revision: "saved", relativePath: "main.tex", kind: "text", content: "source", byteSize: 6};
const proposal = {schemaVersion: 1, manuscriptId, revision: "saved", proposalId: "proposal", changeCount: 2};
const cases: readonly {name: string; call(gateway: ManuscriptVersionGateway): Promise<unknown>;
  command: string; args: unknown; nativeResult: unknown; expectedResult: unknown}[] = [
  {name: "initialize", call: g => g.initialize(manuscriptId), command: "agent_initialize_manuscript_git_repository",
    args: {request: {manuscriptId}}, nativeResult: repository, expectedResult: repository},
  {name: "listBranches", call: g => g.listBranches(manuscriptId), command: "agent_list_manuscript_git_branches",
    args: {manuscriptId}, nativeResult: branches, expectedResult: branches},
  {name: "createBranch", call: g => g.createBranch(manuscriptId, "feature"), command: "agent_create_manuscript_git_branch",
    args: {request: {manuscriptId, branch: "feature"}}, nativeResult: branches, expectedResult: branches},
  {name: "deleteBranch", call: g => g.deleteBranch(manuscriptId, "feature"), command: "agent_delete_manuscript_git_branch",
    args: {request: {manuscriptId, branch: "feature"}}, nativeResult: branches, expectedResult: branches},
  {name: "diffVersions", call: g => g.diffVersions(manuscriptId, "before", "after"), command: "agent_diff_manuscript_git_versions",
    args: {request: {manuscriptId, fromRevision: "before", toRevision: "after"}}, nativeResult: diff, expectedResult: diff},
  {name: "list", call: g => g.list(manuscriptId), command: "agent_list_manuscript_git_versions",
    args: {request: {manuscriptId}}, nativeResult: history, expectedResult: history},
  {name: "list with limit", call: g => g.list(manuscriptId, 7), command: "agent_list_manuscript_git_versions",
    args: {request: {manuscriptId, limit: 7}}, nativeResult: history, expectedResult: history},
  {name: "create", call: g => g.create(manuscriptId, "saved version"), command: "agent_create_manuscript_git_version",
    args: {request: {manuscriptId, message: "saved version"}}, nativeResult: {commit}, expectedResult: commit},
  {name: "readFile", call: g => g.readFile(manuscriptId, "saved", "main.tex"), command: "agent_read_manuscript_git_version_file",
    args: {request: {manuscriptId, revision: "saved", relativePath: "main.tex"}}, nativeResult: file, expectedResult: file},
  {name: "restoreVersion", call: g => g.restoreVersion(scope, "saved"), command: "agent_restore_manuscript_git_version",
    args: {request: {...scope, revision: "saved"}}, nativeResult: proposal, expectedResult: proposal},
  {name: "latestRestoreScope", call: g => g.latestRestoreScope("conversation", manuscriptId), command: "agent_latest_manuscript_git_restore_scope",
    args: {request: {conversationId: "conversation", manuscriptId}}, nativeResult: scope, expectedResult: scope},
  {name: "latestRestoreScope missing", call: g => g.latestRestoreScope("conversation", manuscriptId), command: "agent_latest_manuscript_git_restore_scope",
    args: {request: {conversationId: "conversation", manuscriptId}}, nativeResult: null, expectedResult: null},
];
function select(mode: AppRuntime["mode"], gateways: AppRuntime["gateways"] = {}): void {
  selectAppRuntime({mode, storage, gateways, dispose() {}});
}
beforeEach(() => {native.invoke.mockReset(); select("browser");});
afterEach(() => select("browser"));

it.each(cases)("retains the exact desktop command, arguments and result for $name", async test => {
  select("desktop");
  native.invoke.mockResolvedValueOnce(test.nativeResult);
  await expect(test.call(getManuscriptVersionGateway())).resolves.toBe(test.expectedResult);
  expect(native.invoke).toHaveBeenCalledExactlyOnceWith(test.command, test.args);
});

it.each(cases)("preserves the native failure identity for $name", async test => {
  select("desktop");
  const failure = {code: "original_git_error", requestId: "request-7", message: "preserved failure"};
  native.invoke.mockRejectedValueOnce(failure);
  await expect(test.call(getManuscriptVersionGateway())).rejects.toBe(failure);
});

it("exposes a stable fixed-method desktop port and rejects browser operations without invoking native", async () => {
  select("desktop");
  const desktop = getManuscriptVersionGateway();
  expect(getManuscriptVersionGateway()).toBe(desktop);
  expect(Object.keys(desktop).sort()).toEqual(["create", "createBranch", "deleteBranch", "diffVersions", "initialize",
    "latestRestoreScope", "list", "listBranches", "readFile", "restoreVersion"]);
  select("browser");
  const browser = getManuscriptVersionGateway();
  for (const test of cases) await expect(test.call(browser)).rejects.toThrow("Git 版本只在桌面应用中可用");
  expect(native.invoke).not.toHaveBeenCalled();
});

it("uses the selected preview instance and fails closed when the override is missing", async () => {
  const override: ManuscriptVersionGateway = {...getManuscriptVersionGateway(), list: vi.fn(async () => history)};
  select("preview", {getManuscriptVersionGateway: override});
  expect(getManuscriptVersionGateway()).toBe(override);
  await expect(getManuscriptVersionGateway().list(manuscriptId, 7)).resolves.toBe(history);
  expect(override.list).toHaveBeenCalledWith(manuscriptId, 7);
  select("preview");
  expect(() => getManuscriptVersionGateway()).toThrow("预览未覆盖业务网关：getManuscriptVersionGateway");
  expect(native.invoke).not.toHaveBeenCalled();
});
