/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {invoke} from "@tauri-apps/api/core";
import {WritingDiffDemoStorage} from "../../../app/previews/agent/WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "../../../app/previews/agent/WritingDiffDemoAgentStorage";
import {createWritingDiffDemoGateway} from "../../../app/previews/agent/writingDiffDemoGateway";
import {useWritingDocuments, type WritingDocumentsOptions} from "../../../../../../plugins/writing_projects/src/features/writing/persistence/useWritingDocuments";

vi.mock("@tauri-apps/api/core", () => ({isTauri: () => false, invoke: vi.fn(async () => {throw new Error("Unexpected desktop I/O");})}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const cleanup of cleanups.splice(0).reverse()) await cleanup(); expect(invoke).not.toHaveBeenCalled(); vi.restoreAllMocks();});

function fixture() {
  const manuscriptId = crypto.randomUUID();
  const storage = new WritingDiffDemoStorage(manuscriptId, [{path: "main.tex", kind: "modified", before: "Original\n", after: "Updated\n", additions: 1, deletions: 1}]);
  const agent = new WritingDiffDemoAgentStorage(storage, undefined);
  const gateway = createWritingDiffDemoGateway(storage, manuscriptId);
  const options: WritingDocumentsOptions = {identity: crypto.randomUUID(), manuscriptId, gateway, checkpointGateway: storage.checkpoints,
    agentGateways: agent.gateways, initialOpenPaths: ["main.tex"], persistWorkspaceSession: false};
  cleanups.push(async () => agent.history.dispose());
  return {storage, gateway, options};
}

async function mount(options: WritingDocumentsOptions) {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let documents!: ReturnType<typeof useWritingDocuments>;
  function Harness({value}: {value: WritingDocumentsOptions}): null {documents = useWritingDocuments(value); return null;}
  const render = (value: WritingDocumentsOptions) => act(async () => root.render(<Harness value={value}/>));
  await render(options);
  cleanups.push(async () => {await act(async () => root.unmount()); host.remove();});
  return {get: () => documents, render};
}
async function settled(assertion: () => void): Promise<void> {
  await vi.waitFor(async () => {await act(async () => Promise.resolve()); assertion();}, {timeout: 3000});
}

it("offers an explicit retry when the initial file list read fails", async () => {
  const f = fixture();
  const list = vi.spyOn(f.gateway, "listFiles").mockRejectedValueOnce({code: "unavailable"});
  const h = await mount(f.options);
  await settled(() => expect(h.get().loadFailure).toEqual({cause: {code: "unavailable"}, retryable: true}));
  expect(list).toHaveBeenCalledTimes(1);
  await act(async () => h.get().retryLoad());
  await settled(() => expect(h.get().ready).toBe(true));
  expect(list).toHaveBeenCalledTimes(2);
  expect(h.get().loadFailure).toBeNull();
  expect(h.get().content).toBe("Original\n");
});

it("resumes a failed read without reopening an existing session", async () => {
  const f = fixture(); await f.storage.files.createFile(f.options.manuscriptId, "second.tex", "Second\n");
  const read = f.gateway.readFile, failure = {code: "unavailable"};
  const reads = vi.spyOn(f.gateway, "readFile").mockImplementation(async (id, path) => {
    if (path === "second.tex" && reads.mock.calls.filter(call => call[1] === path).length === 1) throw failure;
    return read(id, path);
  });
  const h = await mount({...f.options, initialOpenPaths: ["main.tex", "second.tex"]});
  await settled(() => expect(h.get().loadFailure).toEqual({cause: failure, retryable: true}));
  const files = h.get().files, original = files.get({manuscriptId: f.options.manuscriptId, relativePath: "main.tex"});
  expect(original?.read().content).toBe("Original\n"); expect(h.get().ready).toBe(false);
  await act(async () => h.get().retryLoad());
  await settled(() => expect(h.get().ready).toBe(true));
  expect(h.get().files).toBe(files);
  expect(files.get({manuscriptId: f.options.manuscriptId, relativePath: "main.tex"})).toBe(original);
  expect(reads.mock.calls.filter(call => call[1] === "main.tex")).toHaveLength(1);
  expect(reads.mock.calls.filter(call => call[1] === "second.tex")).toHaveLength(2);
  expect(h.get().loadFailure).toBeNull();
});

it("does not show a stale owner's failure on a replacement file owner", async () => {
  const first = fixture(), next = fixture(); let reject!: (cause: unknown) => void;
  const pending = new Promise<never>((_resolve, fail) => {reject = fail;});
  vi.spyOn(first.gateway, "listFiles").mockReturnValueOnce(pending);
  const h = await mount(first.options); await h.render(next.options);
  await settled(() => expect(h.get().ready).toBe(true));
  await act(async () => {reject({code: "unavailable"}); await Promise.resolve();});
  expect(h.get().loadFailure).toBeNull(); expect(h.get().ready).toBe(true);
  expect(h.get().content).toBe("Original\n");
});

it("surfaces checkpoint recovery failure without retrying it automatically or through file-read retry", async () => {
  const f = fixture(), failure = {code: "checkpoint_unavailable"};
  const list = vi.spyOn(f.storage.checkpoints, "list").mockRejectedValue(failure);
  const h = await mount(f.options);
  await settled(() => expect(h.get().loadFailure).toEqual({cause: failure, retryable: false}));
  const attempts = list.mock.calls.length;
  await act(async () => h.get().retryLoad());
  expect(list).toHaveBeenCalledTimes(attempts);
  expect(h.get().ready).toBe(false);
});
