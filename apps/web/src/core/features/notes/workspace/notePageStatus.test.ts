// @vitest-environment happy-dom
import {beforeEach, expect, it, vi} from "vitest";
import {NotesGatewayError} from "@research-assistant/data/notes";
import {NotesClient} from "../NotesClient";
import {browserNotesGateway as gateway} from "../gateways/browserNotesGateway";
import {selectAppStorage} from "../../../runtime/appStorage";
import {getReaderCloseCoordinator} from "../../../workspace/DocumentCloseCoordinator";
import {applyNotePageStatus, NOTE_METADATA_CHANGED, type NoteMetadataChanged} from "../../../../../../../plugins/note_manager/src/features/notes/workspace/noteFavoriteActions";

beforeEach(() => {localStorage.clear(); selectAppStorage(localStorage);});

it("publishes a single committed cascade receipt from the toolbar path and can restore without changing identity", async () => {
  const parent = await gateway.createPage({title: "parent"});
  const child = await gateway.createPage({title: "child", parentPageId: parent.page.id});
  const notifications: NoteMetadataChanged[] = [];
  const listener = (event: Event) => notifications.push((event as CustomEvent<NoteMetadataChanged>).detail);
  window.addEventListener(NOTE_METADATA_CHANGED, listener);
  const client = new NotesClient(gateway), prepare = vi.fn(async () => true);
  const unregister = getReaderCloseCoordinator().register("status-cascade", {prepare, isActive: () => false, commit: () => {}});
  try {
    const trashed = await applyNotePageStatus(client, parent.page.id, "trashed");
    expect(trashed).toMatchObject({id: parent.page.id, status: "trashed"});
    expect(prepare).toHaveBeenCalledOnce(); expect(notifications).toHaveLength(1);
    expect(notifications[0]!.pages.filter(page => page.status === "trashed").map(page => page.id).sort())
      .toEqual([parent.page.id, child.page.id].sort());
    const restored = await applyNotePageStatus(client, parent.page.id, "active");
    expect(restored).toMatchObject({id: parent.page.id, status: "active"});
    expect((await gateway.loadDocument(child.page.id)).page.status).toBe("active");
    const unchanged = await applyNotePageStatus(client, parent.page.id, "active");
    expect(unchanged).toEqual(restored); expect(notifications).toHaveLength(2);
  } finally {unregister(); window.removeEventListener(NOTE_METADATA_CHANGED, listener);}
});

it("does not publish or mutate when pre-delete preservation is blocked", async () => {
  const current = await gateway.createPage({title: "keep"}), apply = vi.fn(gateway.applyMetadataBatch);
  const event = vi.fn(); window.addEventListener(NOTE_METADATA_CHANGED, event);
  const unregister = getReaderCloseCoordinator().register("status-blocked", {prepare: async () => false, isActive: () => false, commit: () => {}});
  try {
    await expect(applyNotePageStatus(new NotesClient({...gateway, applyMetadataBatch: apply}), current.page.id, "trashed")).rejects.toThrow("保存");
    expect(apply).not.toHaveBeenCalled(); expect(event).not.toHaveBeenCalled();
    expect((await gateway.loadDocument(current.page.id)).page.status).toBe("active");
  } finally {unregister(); window.removeEventListener(NOTE_METADATA_CHANGED, event);}
});

it("preserves a failed revision cause and never emits a successful deletion event", async () => {
  const current = await gateway.createPage({title: "conflict"}), event = vi.fn();
  const client = new NotesClient({...gateway, applyMetadataBatch: async request => ({operationId: request.operationId,
    succeededIds: [], pages: [], bodyChangedIds: [], favoritePageIds: [], failures: [{id: current.page.id,
      code: "revision_conflict", message: "another writer changed the page", currentRevision: 9}]})});
  window.addEventListener(NOTE_METADATA_CHANGED, event);
  try {
    await expect(applyNotePageStatus(client, current.page.id, "trashed")).rejects.toMatchObject({
      code: "revision_conflict", currentRevision: 9, message: expect.stringContaining("another writer changed the page"),
    });
    expect(event).not.toHaveBeenCalled();
    expect((await gateway.loadDocument(current.page.id)).page.status).toBe("active");
  } finally {window.removeEventListener(NOTE_METADATA_CHANGED, event);}
});

it("does not turn a transaction failure into a committed deletion", async () => {
  const current = await gateway.createPage({title: "storage"}), event = vi.fn();
  const client = new NotesClient({...gateway, applyMetadataBatch: async () => {throw new NotesGatewayError("storage_error", "commit failed");}});
  window.addEventListener(NOTE_METADATA_CHANGED, event);
  try {
    await expect(applyNotePageStatus(client, current.page.id, "trashed")).rejects.toMatchObject({code: "storage_error"});
    expect(event).not.toHaveBeenCalled();
  } finally {window.removeEventListener(NOTE_METADATA_CHANGED, event);}
});
