// @vitest-environment happy-dom
import { beforeEach, expect, it, vi } from "vitest";
import { createNoteDocumentFromText, noteBlockId, type NoteIndexItem } from "@research-assistant/core/notes";
import { browserNotesGateway as gateway } from "../../../../../../../apps/web/src/core/features/notes/gateways/browserNotesGateway";
import { selectAppStorage } from "../../../../../../../apps/web/src/core/runtime/appStorage";
import { NotesClient } from "../../../../../../../apps/web/src/core/features/notes/NotesClient";
import { NotesGatewayError } from "@research-assistant/data/notes";
import { applyFavoriteAction, applyNoteBulkAction } from "../../../../../../../../plugins/ra-note-manager/src/features/notes/workspace/noteFavoriteActions";
import { noteBulkDisabledReason } from "../../../../../../../../plugins/ra-note-manager/src/features/notes/workspace/noteBulkActions";
import { getReaderCloseCoordinator } from "../../../../../../../apps/web/src/core/workspace/DocumentCloseCoordinator";
import { isTextEditorHeld, registerNoteReviewSavePort } from "../../../../../../../apps/web/src/core/features/notes/editor/noteTextEditorPorts";

beforeEach(() => { localStorage.clear(); selectAppStorage(localStorage); });
async function item(title: string): Promise<NoteIndexItem & {target: Extract<NoteIndexItem["target"], {kind: "page"}>}> {
  const {page} = await gateway.createPage({title});
  return {id: page.id, title, snippet: "", updatedAt: page.updatedAt, category: "independent", status: "active", isFavorite: false, target: {kind: "page", pageId: page.id}};
}
it("soft-deletes and restores original documents with fresh revisions and one shared save guard", async () => {
  const items = [await item("一"), await item("二")], client = new NotesClient(gateway);
  const before = await gateway.saveDocument({saveReason: "manual", expectedRevision: 0, document: createNoteDocumentFromText(items[0]!.target.pageId, "需要保留的正文", () => noteBlockId(crypto.randomUUID()))});
  const prepare = vi.fn(async () => true), stop = getReaderCloseCoordinator().register("batch-test", {prepare, isActive: () => false, commit: () => {}});
  try {
    const deleted = await applyNoteBulkAction(client, items, "trash");
    expect(deleted).toEqual({succeededIds: items.map(item => item.id), failures: []}); expect(prepare).toHaveBeenCalledOnce();
    const trashed = await gateway.loadDocument(before.page.id);
    expect(trashed.page.status).toBe("trashed"); expect(trashed.document).toEqual(before.document);
    await applyNoteBulkAction(client, items.map(item => ({...item, status: "trashed"})), "restore");
    expect((await gateway.loadDocument(before.page.id)).page.status).toBe("active");
    expect(isTextEditorHeld(before.page.id)).toBe(false);
  } finally { stop(); }
});

it("collects partial failures without discarding original causes or retrying successful notes", async () => {
  const items = [await item("成功"), await item("冲突"), await item("后续")];
  const save = vi.fn<typeof gateway.applyMetadataBatch>(request => gateway.applyMetadataBatch({...request, items: request.items.map(item => item.id === items[1]!.id ? {...item, expectedRevision: item.expectedRevision + 1} : item)}));
  const result = await applyNoteBulkAction(new NotesClient({...gateway, applyMetadataBatch: save}), items, "archive");
  expect(result.succeededIds).toEqual([items[0]!.id, items[2]!.id]);
  expect(result.failures).toMatchObject([{id: items[1]!.id, title: "冲突", code: "revision_conflict", currentRevision: 0}]);
  expect(save).toHaveBeenCalledOnce(); expect(items.every(item => !isTextEditorHeld(item.id))).toBe(true);
});

it("blocks unsupported mixed selections before any mutation, including dependent favorites", async () => {
  const ordinary = await item("普通"), writing: NoteIndexItem = {...ordinary, id: "writing", target: {kind: "writing", manuscriptId: "project", path: "note.ranote"}};
  const mutate = vi.fn(gateway.applyMetadataBatch), client = new NotesClient({...gateway, applyMetadataBatch: mutate});
  await expect(applyNoteBulkAction(client, [ordinary, writing], "trash")).rejects.toThrow("写作项目");
  await expect(applyNoteBulkAction(client, [{...ordinary, category: "dependent"}], "favorite")).rejects.toThrow("附属笔记");
  expect(mutate).not.toHaveBeenCalled();
  expect(noteBulkDisabledReason([{...ordinary, category: "dependent"}], "archive")).toBeUndefined();
});

it("does not write when preserving current editors fails, and does not change edit times for favorites", async () => {
  const ordinary = await item("收藏"), client = new NotesClient(gateway), prepare = vi.fn(async () => false);
  const stop = getReaderCloseCoordinator().register("blocked-batch", {prepare, isActive: () => false, commit: () => {}});
  try { await expect(applyNoteBulkAction(client, [ordinary], "trash")).rejects.toThrow("保存"); }
  finally { stop(); }
  await applyNoteBulkAction(client, [ordinary], "favorite");
  const current = await gateway.loadDocument(ordinary.target.pageId);
  expect(current.page.isFavorite).toBe(true); expect(current.page.updatedAt).toBe(ordinary.updatedAt); expect(current.page.status).toBe("active");
});

it("retains gateway error codes and revision diagnostics in bulk and single-row actions", async () => {
  const ordinary = await item("诊断"), error = new NotesGatewayError("revision_conflict", "基线发生变化", 17);
  const client = new NotesClient({...gateway, applyMetadataBatch: async request => ({operationId: request.operationId, succeededIds: [], pages: [], bodyChangedIds: [], favoritePageIds: [], failures: [{id: ordinary.target.pageId, code: error.code, message: error.message, currentRevision: 17}]})});
  const result = await applyNoteBulkAction(client, [ordinary], "archive");
  expect(result.failures[0]!.message).toContain("revision_conflict");
  expect(result.failures[0]!.message).toContain("17");
  await expect(applyFavoriteAction(client, ordinary, "archive")).rejects.toMatchObject({code: error.code, currentRevision: 17});
  expect(isTextEditorHeld(ordinary.id)).toBe(false);
});
it("keeps committed successes when subsequent open-editor synchronization fails", async () => {
  const ordinary = await item("已提交"), client = new NotesClient(gateway);
  const stop = registerNoteReviewSavePort(ordinary.target.pageId, {flush: async () => {}, retry: async () => {}, getState: () => ({status: "saved"}),
    synchronize: async () => {throw new NotesGatewayError("storage_error", "同步断开");}});
  try {
    const result = await applyNoteBulkAction(client, [ordinary], "trash");
    expect(result.succeededIds).toEqual([ordinary.id]); expect(result.failures).toEqual([]);
    expect(result.warnings?.[0]).toContain("已提交");
    expect(result.warnings?.[0]).toContain("storage_error");
    expect((await gateway.listPageMetadata([ordinary.target.pageId]))[0]?.status).toBe("trashed");
    expect(isTextEditorHeld(ordinary.target.pageId)).toBe(false);
  } finally {stop();}
});
it("keeps the previous per-item favorite insertion order in one batch", async () => {
  const first = await item("first"), second = await item("second");
  await applyNoteBulkAction(new NotesClient(gateway), [first, second], "favorite");
  expect(await gateway.listFavoritePageIds()).toEqual([second.target.pageId, first.target.pageId]);
});
