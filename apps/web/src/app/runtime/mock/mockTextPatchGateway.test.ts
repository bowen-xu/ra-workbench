/** @vitest-environment happy-dom */
import { expect, it, vi } from "vitest";
import { createMockTextPatchGateway, TEXT_PATCH_PREVIEW_CONVERSATION, TEXT_PATCH_PREVIEW_PAGE } from "./mockTextPatchGateway";
import type { TextPatchDecision, TextPatchResolution } from "../../../core/features/agent/textPatchGateway";
import type { NoteDocumentSnapshot } from "@research-assistant/data/notes";

function setup() {
  const gateway = createMockTextPatchGateway(true);
  const install = vi.fn(() => undefined);
  const prepare = vi.fn(() => install);
  gateway.bindEditor({ isInstalling: false, read: () => gateway.snapshot().document, prepare, prepareStructure: prepare, dispose() {} });
  const decision = (action: TextPatchDecision["action"], requestId = crypto.randomUUID()): TextPatchDecision => {
    const review = gateway.snapshot().review;
    if (!review) throw new Error("proposal missing");
    return { conversationId: TEXT_PATCH_PREVIEW_CONVERSATION, proposalId: review.proposalId, generation: review.generation, requestId, action };
  };
  const resolution = (action: TextPatchResolution["action"], extra: Partial<TextPatchResolution> = {}): TextPatchResolution => {
    const review = gateway.snapshot().review;
    if (!review) throw new Error("proposal missing");
    return { conversationId: TEXT_PATCH_PREVIEW_CONVERSATION, proposalId: review.proposalId, generation: review.generation,
      requestId: crypto.randomUUID(), pageId: TEXT_PATCH_PREVIEW_PAGE, action, ...extra };
  };
  return { gateway, install, prepare, decision, resolution };
}
it("reconciles a stale mirror idempotently without rewriting confirmed content or revision",async()=>{
 const {gateway,resolution,install}=setup();gateway.simulateStaleProposal();
 const before=gateway.snapshot(),request=resolution("retain-current",{document:before.document,expectedRevision:before.revision});
 const receipt=await gateway.resolve(request);expect(receipt.pages).toEqual([]);expect(await gateway.resolve(request)).toEqual(receipt);
 expect(gateway.snapshot().document).toEqual(before.document);expect(gateway.snapshot().revision).toBe(before.revision);expect(install).not.toHaveBeenCalled();
 expect(gateway.snapshot().review?.baselineMatches).toBe(true);
});
it("saves current D while retaining B and preserves the existing address and title", async () => {
  const { gateway } = setup();
  const initial = gateway.read("saved"), document = gateway.snapshot().document;
  await gateway.simulate("rewrite");
  expect(gateway.snapshot().document).not.toBe(document);
  expect(gateway.read("saved")).not.toBe(initial);
  expect(gateway.read("saved")).toBe(gateway.read("candidate"));
  expect(gateway.snapshot().review?.pages[0]?.reviewDocument).toBe(document);
  expect(gateway.read("candidate")).toContain("证据更新判断");
  const result = gateway.snapshot().messages.find(item => item.toolName === "research_apply_patch" && item.kind === "tool-result");
  expect(result?.content).toContain("source://notes/00000000-0000-4000-8000-000000000022");
  expect(result?.content).toContain("研究笔记");
  expect(result?.content).not.toContain(".md");
  expect(gateway.read("candidate")).toContain('<reference kind="paper"');
  expect(gateway.read("candidate")).toContain("[方法说明](https://example.org/method)");
});
it("continues the same group while retaining the baseline and rejects an outdated decision", async () => {
  const { gateway, decision } = setup();
  await gateway.simulate("rewrite"); const first = gateway.snapshot().review, oldDecision = decision("accept");
  await gateway.simulate("checklist"); const next = gateway.snapshot().review;
  expect(next?.proposalId).toBe(first?.proposalId);
  expect(next?.generation).toBe(2);
  expect(next?.pages[0]?.before).toBe(first?.pages[0]?.before);
  expect(gateway.read("candidate")).toContain("下一步验证");
  await expect(gateway.decide(oldDecision)).rejects.toMatchObject({ status: "failed" });
});
it("installs once for repeated requests and restores the baseline when undone", async () => {
  const { gateway, decision, install, prepare } = setup();
  const initial = gateway.snapshot().document;
  await gateway.simulate("rewrite"); await gateway.simulate("checklist");
  const request = decision("accept");
  await gateway.decide(request); await gateway.decide(request);
  expect(install).toHaveBeenCalledTimes(2);
  expect(prepare).toHaveBeenCalledWith(initial, expect.anything(), expect.objectContaining({ origin: "agent" }));
  expect(gateway.read("saved")).toContain("下一步验证");
  await gateway.decide(decision("undo"));
  expect(install).toHaveBeenCalledTimes(3);
  expect(gateway.snapshot().document).toBe(initial);
  expect(gateway.read("saved")).not.toContain("下一步验证");
});
it("reverses saved D when rejecting a pending review", async () => {
  const { gateway, decision, install } = setup();
  const document = gateway.snapshot().document;
  await gateway.simulate("rewrite"); await gateway.decide(decision("reject"));
  expect(install).toHaveBeenCalledTimes(2);
  expect(gateway.snapshot().document).toBe(document);
  expect(gateway.snapshot().revision).toBe(3);
  expect(gateway.read("candidate")).toBe(gateway.read("saved"));
});
it("preserves manual edits, blocks stale acceptance and permits rejection", async () => {
  const { gateway, decision, install } = setup();
  await gateway.simulate("rewrite");
  const edited = { ...gateway.snapshot().document, blocks: [] };
  gateway.edit(edited);
  await expect(gateway.decide(decision("accept"))).rejects.toMatchObject({ status: "failed" });
  expect(gateway.snapshot().document).toBe(edited);
  expect(install).toHaveBeenCalledOnce();
  await gateway.decide(decision("reject"));
  expect(gateway.snapshot().document).toBe(edited);
  expect(gateway.read("saved")).not.toContain("逻辑推理模型");
});
it("fails closed without a mounted editor and resets disposable state", async () => {
  const { gateway, decision } = setup();
  await gateway.simulate("rewrite"); gateway.bindEditor(undefined);
  // Baseline-only Keep requires no body write or attached editor.
  await gateway.decide(decision("accept"));
  const session = gateway.snapshot().session;
  gateway.reset();
  expect(gateway.snapshot().session).toBe(session + 1);
  expect(gateway.snapshot().messages).toEqual([]);
  expect(gateway.snapshot().review).toBeNull();
});

it("advances B for a partial Keep without resaving D and saves partial Undo immediately", async () => {
  const { gateway, install, resolution } = setup();
  await gateway.simulate("rewrite"); await gateway.simulate("checklist");
  const formal = gateway.snapshot().document, revision = gateway.snapshot().revision;
  const before = gateway.snapshot().review!;
  expect(before.pages[0]?.chunks).toHaveLength(2);
  const keep = resolution("keep", { chunkId: before.pages[0]!.chunks![0]!.id });
  const first = await gateway.resolve(keep);
  expect(first.pages).toEqual([{pageId: TEXT_PATCH_PREVIEW_PAGE, revision: revision + 1, document: formal}]);
  expect(first.review?.pages[0]?.reviewDocument?.blocks[1]?.content).toEqual(before.pages[0]?.afterDocument?.blocks[1]?.content);
  expect(gateway.snapshot().document).toBe(formal);
  expect(gateway.snapshot().revision).toBe(revision + 1);
  expect(install).toHaveBeenCalledTimes(2);
  expect(first.review?.pages[0]?.chunks).toHaveLength(1);
  const undo = resolution("undo", { chunkId: first.review!.pages[0]!.chunks![0]!.id });
  const done = await gateway.resolve(undo);
  expect(done.review?.state).toBe("accept");
  expect(gateway.read("saved")).toContain("证据更新判断");
  expect(gateway.read("saved")).not.toContain("下一步验证");
  expect(install).toHaveBeenCalledTimes(3);
  expect(await gateway.resolve(keep)).toBe(first);
  expect(install).toHaveBeenCalledTimes(3);
});

it("updates ordinary roots in B and D, keeps pending old content, and preserves manual input in the next patch", async () => {
  const { gateway, resolution } = setup();
  await gateway.simulate("rewrite");
  const candidate = gateway.snapshot().review!.pages[0]!.afterDocument!;
  const edited = { ...candidate, blocks: candidate.blocks.map((block, index) => index < 2
    ? { ...block, content: { kind: "inline" as const, items: [{ type: "text" as const, text: index ? "模型已有手动补充。" : "用户的研究标题", styles: [] }] } } : block) };
  await gateway.resolve(resolution("edit", { document: edited }));
  const review = gateway.snapshot().review!;
  expect(review.pages[0]?.chunks).toHaveLength(1);
  expect(review.pages[0]?.reviewDocument?.blocks[0]?.content).toEqual(edited.blocks[0]?.content);
  expect(review.pages[0]?.reviewDocument?.blocks[1]?.content).toEqual(review.pages[0]?.beforeDocument?.blocks[1]?.content);
  expect(review.wholeUndoAvailable).toBe(false);
  await gateway.simulate("checklist");
  expect(gateway.read("candidate")).toContain("用户的研究标题");
  expect(gateway.read("candidate")).toContain("模型已有手动补充。");
  expect(gateway.read("candidate")).toContain("下一步验证");
  expect(gateway.snapshot().review?.pages[0]?.reviewDocument?.blocks[0]?.content).toEqual(edited.blocks[0]?.content);
  expect(gateway.read("saved")).toContain("用户的研究标题");
  await gateway.resolve(resolution("keep"));
  expect(gateway.read("saved")).toContain("模型已有手动补充。");
});

it("restores Keep history without rewinding S and repeats baseline undo/redo safely", async () => {
  const { gateway, resolution, install } = setup();
  await gateway.simulate("rewrite");
  const keep = resolution("keep");
  await gateway.resolve(keep);
  const saved = gateway.snapshot().document, revision = gateway.snapshot().revision;
  for (let cycle = 0; cycle < 2; cycle++) {
    const undone = await gateway.resolve(resolution("restore", { historyRequestId: keep.requestId, direction: "undo" }));
    expect(undone.review?.state).toBe("pending");
    expect(undone.review?.pages[0]?.chunks).toHaveLength(1);
    expect(gateway.snapshot().document).toBe(saved);
    expect(gateway.snapshot().revision).toBe(revision + cycle * 2 + 1);
    const redone = await gateway.resolve(resolution("restore", { historyRequestId: keep.requestId, direction: "redo" }));
    expect(redone.review?.pages[0]?.chunks).toEqual([]);
    expect(redone.review?.state).toBe("accept");
    expect(gateway.snapshot().revision).toBe(revision + cycle * 2 + 2);
  }
  expect(install).toHaveBeenCalledOnce();
});

it("freezes chat tool generations independently of later B/D decisions and rejects changed request identities", async () => {
  const { gateway, resolution } = setup();
  await gateway.simulate("rewrite");
  const first = gateway.snapshot().review!;
  await gateway.simulate("checklist");
  const frozen = await gateway.review(TEXT_PATCH_PREVIEW_CONVERSATION, first.proposalId, first.generation);
  expect(frozen?.pages[0]?.after).not.toContain("下一步验证");
  expect(frozen?.operationGeneration).toBe(gateway.snapshot().review?.generation);
  const second = await gateway.review(TEXT_PATCH_PREVIEW_CONVERSATION, first.proposalId, gateway.snapshot().review!.generation);
  expect(second?.pages[0]?.before).toBe(first.pages[0]?.after);
  const keep = resolution("keep");
  await gateway.resolve(keep);
  const history = await gateway.review(TEXT_PATCH_PREVIEW_CONVERSATION, first.proposalId, first.generation);
  expect(history?.pages[0]?.after).toBe(first.pages[0]?.after);
  expect(history?.state).toBe("accept");
  await expect(gateway.resolve({ ...keep, action: "undo" })).rejects.toMatchObject({ status: "failed", message: "请求身份冲突" });
});

it("deduplicates concurrent requests, rejects a second owner, and never recursively flushes its host", async () => {
  const { gateway, resolution } = setup();
  let finish: ((snapshot: NoteDocumentSnapshot) => void) | undefined;
  const apply = vi.fn(() => new Promise<NoteDocumentSnapshot>(resolve => { finish = resolve; }));
  const flush = vi.fn(async () => undefined);
  await gateway.simulate("rewrite"); gateway.connectHost({ apply, flush });
  const request = resolution("undo"), first = gateway.resolve(request), repeat = gateway.resolve(request);
  await vi.waitFor(() => expect(apply).toHaveBeenCalledOnce());
  await expect(gateway.resolve(resolution("keep"))).rejects.toMatchObject({ status: "failed", message: "笔记审阅操作正在处理，请等待原操作结果。" });
  finish!({ page: { revision: 3 }, document: gateway.snapshot().review!.pages[0]!.beforeDocument } as NoteDocumentSnapshot);
  const [left, right] = await Promise.all([first, repeat]);
  expect(left).toBe(right);
  expect(apply).toHaveBeenCalledOnce(); expect(flush).not.toHaveBeenCalled();
});

it("rolls back candidate resolution on formal save failure and retries the original identity", async () => {
  const { gateway, resolution } = setup();
  const apply = vi.fn(async (document, expectedRevision: number) => ({ page: { revision: expectedRevision + 1 }, document } as NoteDocumentSnapshot));
  apply.mockRejectedValueOnce(new Error("storage failure"));
  await gateway.simulate("rewrite"); gateway.connectHost({ apply, flush: async () => undefined });
  const previous = gateway.snapshot().review, request = resolution("undo");
  await expect(gateway.resolve(request)).rejects.toThrow("storage failure");
  expect(gateway.snapshot().review).toBe(previous); expect(gateway.snapshot().revision).toBe(2);
  expect(gateway.snapshot().review?.pages[0]?.chunks).toHaveLength(1);
  await gateway.resolve(request); await gateway.resolve(request);
  expect(apply).toHaveBeenCalledTimes(2); expect(gateway.snapshot().revision).toBe(3);
});

it("uses the authoritative page revision after metadata-only changes", async () => {
  const { gateway } = setup();
  let actualRevision = 17;
  gateway.connectHost({
    flush: async () => {},
    readSaved: async () => ({ page: { revision: actualRevision }, document: gateway.snapshot().document }) as import("@research-assistant/data/notes").NoteDocumentSnapshot,
    apply: async (document, expectedRevision) => {
      expect(expectedRevision).toBe(actualRevision);
      actualRevision++;
      return { page: { revision: actualRevision }, document } as import("@research-assistant/data/notes").NoteDocumentSnapshot;
    },
  });
  await gateway.simulate("rewrite");
  expect(gateway.snapshot().revision).toBe(18);
  expect(gateway.snapshot().review?.state).toBe("pending");
});
