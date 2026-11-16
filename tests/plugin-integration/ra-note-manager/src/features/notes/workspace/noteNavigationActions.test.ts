// @vitest-environment happy-dom
import { beforeEach, expect, it, vi } from "vitest";
import { browserNotesGateway as gateway } from "../../../../../../../apps/web/src/core/features/notes/gateways/browserNotesGateway";
import { selectAppStorage } from "../../../../../../../apps/web/src/core/runtime/appStorage";
import { NotesClient } from "../../../../../../../apps/web/src/core/features/notes/NotesClient";
import { moveFavoriteLink } from "../../../../../../../../plugins/ra-note-manager/src/features/notes/workspace/noteNavigationActions";
import { isTextEditorHeld, registerNoteReviewSavePort } from "../../../../../../../apps/web/src/core/features/notes/editor/noteTextEditorPorts";

beforeEach(() => { localStorage.clear(); selectAppStorage(localStorage); });
it("moves a favorite into a body and promotes its specific link back without changing page ownership", async () => {
  const parent = await gateway.createPage({ title: "parent" }), child = await gateway.createPage({ title: "child" });
  await gateway.updatePage({ pageId: child.page.id, expectedRevision: 0, isFavorite: true });
  const client = new NotesClient(gateway);
  await moveFavoriteLink(client, { source: { pageId: child.page.id, title: "child" }, destination: { pageId: parent.page.id, placement: "inside" } });
  const body = await gateway.loadDocument(parent.page.id);
  expect(body.document.blocks.at(-1)?.content).toMatchObject({ kind: "reference", targetId: child.page.id });
  expect(await gateway.listFavoritePageIds()).toEqual([]);
  expect((await gateway.loadDocument(child.page.id)).page.status).toBe("active");
  await moveFavoriteLink(client, { source: { pageId: child.page.id, title: "child", parentPageId: parent.page.id, blockId: body.document.blocks.at(-1)!.id } });
  expect((await gateway.loadDocument(parent.page.id)).document.blocks).toEqual(parent.document.blocks);
  expect(await gateway.listFavoritePageIds()).toEqual([child.page.id]);
  expect((await gateway.loadDocument(child.page.id)).page.parentPageId).toBeUndefined();
  expect((await gateway.loadDocument(child.page.id)).page.status).toBe("active");
  expect((await gateway.loadDocument(child.page.id)).document).toEqual({...child.document, title: "child"});
  expect(isTextEditorHeld(parent.page.id)).toBe(false);
});
it("blocks pending reviews and preserves bodies and all holds after a rejected transaction", async () => {
  const parent = await gateway.createPage({ title: "parent" }), child = await gateway.createPage({ title: "child" });
  const stop = registerNoteReviewSavePort(parent.page.id, { flush: async () => {}, retry: async () => {}, getState: () => ({ status: "saved", pendingChanges: 1 }) });
  const commit = vi.fn(gateway.commitNavigation), client = new NotesClient({ ...gateway, commitNavigation: commit });
  const move = { source: { pageId: child.page.id, title: "child" }, destination: { pageId: parent.page.id, placement: "inside" as const } };
  try { await expect(moveFavoriteLink(client, move)).rejects.toThrow("待审阅"); expect(commit).not.toHaveBeenCalled(); } finally { stop(); }
  const broken = new NotesClient({ ...gateway, commitNavigation: async request => gateway.commitNavigation({ ...request, documents: request.documents.map(save => ({ ...save, expectedRevision: 99 })) }) });
  await expect(moveFavoriteLink(broken, move)).rejects.toMatchObject({ code: "revision_conflict" });
  expect((await gateway.loadDocument(parent.page.id)).document).toEqual({...parent.document, title: parent.page.title});
  expect(isTextEditorHeld(parent.page.id)).toBe(false); expect(isTextEditorHeld(child.page.id)).toBe(false);
});
