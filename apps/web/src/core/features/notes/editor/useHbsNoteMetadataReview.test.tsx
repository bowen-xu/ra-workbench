// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createNoteDocumentAdapter, HbsSession } from "@research-assistant/core/hbs";
import { createNoteDocumentFromText, noteBlockId, notePageId } from "@research-assistant/core/notes";
import { directTextDiff } from "../../../components/hbs/codeMirrorHbsPolicy";
import { useHbsNoteMetadataReview } from "./useHbsNoteMetadataReview";
import { flushNoteReview, getNoteReviewSaveState, registerNoteReviewSavePort, retryNoteReview } from "./noteTextEditorPorts";

const f = vi.hoisted(() => ({ review: vi.fn(), resolve: vi.fn(), save: vi.fn(), clear: vi.fn(), draft: undefined as unknown }));
vi.mock("../../../runtime/appRuntime", () => ({ getAppRuntime: () => ({mode:"preview"}) }));
vi.mock("../../agent/textPatchGateway", () => ({ textPatchGateway: { forPage: async () => ["conversation"], review: f.review, resolve: f.resolve }, subscribeTextPatches: () => () => {} }));
vi.mock("../workspace/noteRecovery", () => ({ saveNoteRecoveryDraft: (...args: unknown[]) => { f.save(...args); f.draft = { review: args[3] }; },
  loadNoteRecoveryDraft: () => f.draft, clearNoteRecoveryDraft: () => { f.clear(); f.draft = undefined; } }));
const stops: (() => Promise<void>)[] = [];
afterEach(async () => { for (const stop of stops.splice(0)) await stop(); vi.resetAllMocks(); f.draft = undefined; vi.unstubAllGlobals(); });
async function setup(candidate = false, metadata = true) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const page = notePageId("page"), base = createNoteDocumentFromText(page, "base", () => noteBlockId("block"));
  const saved = candidate ? createNoteDocumentFromText(page, "new", () => noteBlockId("block")) : base;
  const snapshot = { version: 1, saved, baseline: base };
  const session = await HbsSession.open({ adapter: createNoteDocumentAdapter(directTextDiff), port: { read: async () => snapshot, commit: async () => { throw new Error("unexpected body write"); } },
    preservation: { load: () => undefined, save: () => {} }, requestId: sequence => `body:${sequence}` });
  f.review.mockResolvedValue(metadata ? { proposalId: "proposal", generation: 1, state: "pending", pages: [{ pageId: page, kind: "move", metadataPending: true, chunks: [] }] } : null);
  const host = document.createElement("div"), root = createRoot(host); let current!: ReturnType<typeof useHbsNoteMetadataReview>;
  function Harness() { current = useHbsNoteMetadataReview(session, true); return null; }
  await act(async () => root.render(<Harness/>));
  stops.push(async () => { await act(async () => root.unmount()); session.dispose(); });
  return () => current;
}
it("preserves the actual metadata controls but never consumes pending body review", async () => {
  const get = await setup(true); expect(get().metadataPending).toBe(true); expect(get().metadataKind).toBe("move");
  await act(async () => get().resolve("keep"));
  expect(get().error).toContain("MODEL_METADATA_BODY_PENDING"); expect(f.resolve).not.toHaveBeenCalled(); expect(f.save).not.toHaveBeenCalled();
});
it("persists and explicitly retries the same legacy metadata request before clearing it", async () => {
  const get = await setup(); f.resolve.mockRejectedValueOnce(new Error("unknown native outcome"));
  await act(async () => get().resolve("keep"));
  const original = f.resolve.mock.calls[0]![0]; expect(f.save).toHaveBeenCalledBefore(f.resolve); expect(get().request?.requestId).toBe(original.requestId);
  expect(f.clear).not.toHaveBeenCalled();
  f.resolve.mockImplementationOnce(async () => { f.review.mockResolvedValue(null); return { pages: [] }; });
  await act(async () => get().restore());
  expect(f.resolve.mock.calls[1]![0]).toEqual(original); expect(f.clear).toHaveBeenCalledOnce(); expect(get().metadataPending).toBe(false);
});

it("contributes metadata saving and unknown IDs to the existing header state without replacing its body owner", async () => {
  const get = await setup(); let reject!: (reason: Error) => void;
  f.resolve.mockImplementationOnce(() => new Promise((_resolve,no)=>{reject=no;}));
  const bodyFlush=vi.fn(async()=>{}),bodyRetry=vi.fn(async()=>{});
  let bodyState: import("./noteTextEditorPorts").NoteReviewSaveState={status:"saved"};
  const stop=registerNoteReviewSavePort("page",{getState:()=>bodyState,flush:bodyFlush,retry:bodyRetry});stops.push(async()=>stop());
  await act(async()=>get().resolve("undo"));
  const original=f.resolve.mock.calls[0]![0];
  expect(getNoteReviewSaveState("page").status).toBe("saving");expect(getNoteReviewSaveState("page").message).toContain(original.requestId);
  await expect(flushNoteReview("page")).rejects.toThrow("MODEL_METADATA_SAVING");expect(bodyFlush).not.toHaveBeenCalled();
  await act(async()=>{reject(new Error("metadata transport unknown"));});
  bodyState={status:"failed",message:"MODEL_RESULT_UNKNOWN：正文原请求 body-request-1；输入已保全。"};
  const aggregate=getNoteReviewSaveState("page");expect(aggregate.status).toBe("failed");
  expect(aggregate.message).toContain("正文：MODEL_RESULT_UNKNOWN");expect(aggregate.message).toContain("body-request-1");
  expect(aggregate.message).toContain("页面及关联：MODEL_METADATA_RESULT_UNKNOWN");expect(aggregate.message).toContain(original.requestId);
  expect(getNoteReviewSaveState("page")).toBe(aggregate);
  bodyRetry.mockImplementationOnce(async()=>{bodyState={status:"saved"};});
  f.resolve.mockImplementationOnce(async()=>{f.review.mockResolvedValue(null);return{pages:[]};});
  await act(async()=>retryNoteReview("page"));
  expect(bodyRetry).toHaveBeenCalledOnce();expect(f.resolve.mock.calls[1]![0]).toEqual(original);expect(getNoteReviewSaveState("page").status).toBe("saved");
});

it("adds no state contribution when the actual page has no metadata review",async()=>{
  await setup(false,false);expect(getNoteReviewSaveState("page")).toEqual({status:"saved"});
  expect(f.review).toHaveBeenCalledWith("conversation",undefined,undefined,"page");
});
