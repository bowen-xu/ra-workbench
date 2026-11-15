// @vitest-environment happy-dom
import {beforeEach, expect, it, vi} from "vitest";
import {NotesGatewayError} from "@research-assistant/data/notes";
import {NotesClient} from "../NotesClient";
import {browserNotesGateway as gateway} from "../gateways/browserNotesGateway";
import {selectAppStorage} from "../../../runtime/appStorage";
import {notify} from "../../../services/notifications";
import {applyNotePageStatus, applyNoteBulkAction, NOTE_PAGES_DELETED} from "../../../../../../../plugins/note_manager/src/features/notes/workspace/noteFavoriteActions";

vi.mock("../../../services/notifications", () => ({notify: {success: vi.fn(), error: vi.fn(), update: vi.fn()}}));
beforeEach(() => {localStorage.clear(); selectAppStorage(localStorage); vi.clearAllMocks();});

it("announces a committed toolbar deletion even without an open document host", async () => {
  const page = await gateway.createPage({title: "delete"});
  await applyNotePageStatus(new NotesClient(gateway), page.page.id, "trashed");
  expect(notify.success).toHaveBeenCalledExactlyOnceWith("笔记已移到回收站", expect.objectContaining({id: expect.stringMatching(/^note-trash:/), actions: expect.arrayContaining([
    expect.objectContaining({id: "restore", label: "恢复笔记", icon: expect.anything()}), expect.objectContaining({id: "delete", label: "永久删除", icon: expect.anything()}),
  ])}));
  expect((await gateway.loadDocument(page.page.id)).page.status).toBe("trashed");
});

it("restores from the message after its document host has closed", async () => {
  const page = await gateway.createPage({title: "restore"});
  await applyNotePageStatus(new NotesClient(gateway), page.page.id, "trashed");
  const options = vi.mocked(notify.success).mock.calls[0]![1]!;
  await options.actions!.find(action => action.id === "restore")!.onClick();
  expect((await gateway.loadDocument(page.page.id)).page.status).toBe("active");
  expect(notify.update).toHaveBeenCalledWith(options.id, {message: "已撤销删除", actions: []});
});

it("cancels permanent deletion and rejects a stale message after restoration", async () => {
  const page = await gateway.createPage({title: "keep"}), client = new NotesClient(gateway);
  await applyNotePageStatus(client, page.page.id, "trashed");
  const action = vi.mocked(notify.success).mock.calls[0]![1]!.actions!.find(action => action.id === "delete")!;
  const originalConfirm = window.confirm, confirm = vi.fn(() => false); window.confirm = confirm;
  try {
    await action.onClick(); expect(confirm).toHaveBeenCalledOnce();
    expect((await gateway.loadDocument(page.page.id)).page.status).toBe("trashed");
    await applyNotePageStatus(client, page.page.id, "active");
    await expect(action.onClick()).rejects.toMatchObject({code: "revision_conflict"});
    expect(confirm).toHaveBeenCalledOnce(); expect(notify.update).not.toHaveBeenCalled();
  } finally {window.confirm = originalConfirm;}
});

it("announces a partially successful batch once, preserving its failures", async () => {
  const a = await gateway.createPage({title: "a"}), b = await gateway.createPage({title: "b"});
  const client = new NotesClient({...gateway, applyMetadataBatch: async request => {
    const receipt = await gateway.applyMetadataBatch({...request, items: request.items.slice(0, 1)});
    return {...receipt, failures: [{id: b.page.id, code: "revision_conflict" as const, message: "changed", currentRevision: 9}]};
  }});
  const items = [a, b].map(({page}) => ({id: page.id, title: page.title, snippet: "", category: "independent" as const,
    updatedAt: page.updatedAt, status: page.status, isFavorite: false, target: {kind: "page" as const, pageId: page.id}}));
  const result = await applyNoteBulkAction(client, items, "trash");
  expect(notify.success).toHaveBeenCalledOnce(); expect(result.succeededIds).toEqual([a.page.id]);
  expect(result.failures).toHaveLength(1);
});

it("does not announce success on a rejected deletion or an archive", async () => {
  const page = await gateway.createPage({title: "keep"});
  const rejected = new NotesClient({...gateway, applyMetadataBatch: async () => {throw new NotesGatewayError("storage_error", "rejected");}});
  await expect(applyNotePageStatus(rejected, page.page.id, "trashed")).rejects.toMatchObject({code: "storage_error"});
  expect(notify.success).not.toHaveBeenCalled();
  await applyNotePageStatus(new NotesClient(gateway), page.page.id, "archived");
  expect(notify.success).not.toHaveBeenCalled();
});

it("permanently deletes only after confirmation and publishes the actual removed identity", async () => {
  const page = await gateway.createPage({title: "synthetic permanent-delete fixture"});
  await applyNotePageStatus(new NotesClient(gateway), page.page.id, "trashed");
  const options = vi.mocked(notify.success).mock.calls[0]![1]!;
  const originalConfirm = window.confirm; window.confirm = vi.fn(() => true);
  const deleted = vi.fn(); window.addEventListener(NOTE_PAGES_DELETED, deleted);
  try {
    await options.actions!.find(action => action.id === "delete")!.onClick();
    await expect(gateway.loadDocument(page.page.id)).rejects.toMatchObject({code: "not_found"});
    expect(deleted).toHaveBeenCalledOnce();
    expect((deleted.mock.calls[0]![0] as CustomEvent).detail).toEqual([page.page.id]);
    expect(notify.update).toHaveBeenCalledWith(options.id, {message: "笔记已永久删除", actions: []});
  } finally {window.confirm = originalConfirm; window.removeEventListener(NOTE_PAGES_DELETED, deleted);}
});
