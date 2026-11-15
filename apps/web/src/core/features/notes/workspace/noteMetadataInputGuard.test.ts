// @vitest-environment happy-dom
import {beforeEach, expect, it, vi} from "vitest";
import {createNoteDocumentFromText, noteBlockId} from "@research-assistant/core/notes";
import {NotesGatewayError} from "@research-assistant/data/notes";
import {NotesClient} from "../NotesClient";
import {browserNotesGateway as gateway} from "../gateways/browserNotesGateway";
import {selectAppStorage} from "../../../runtime/appStorage";
import {getReaderCloseCoordinator} from "../../../workspace/DocumentCloseCoordinator";
import {isTextEditorHeld, isTextEditorSaveBlocked, registerTextInstallPort} from "../editor/noteTextEditorPorts";
import {applyNotePageStatus} from "../../../../../../../plugins/note_manager/src/features/notes/workspace/noteFavoriteActions";

beforeEach(() => {localStorage.clear(); selectAppStorage(localStorage);});
function deferred() {let resolve!: () => void; const promise = new Promise<void>(yes => {resolve = yes;}); return {promise, resolve};}
async function fixture() {
  const parent = await gateway.createPage({title: "parent"}), child = await gateway.createPage({title: "child", parentPageId: parent.page.id});
  let live = createNoteDocumentFromText(child.page.id, "latest unsaved child", () => noteBlockId("child-body")), locked = false;
  const setWriteLocked = vi.fn((value: boolean) => {locked = value;});
  const stop = registerTextInstallPort(child.page.id, {read: () => live, setWriteLocked, isInstalling: false,
    prepare: () => () => undefined, prepareStructure: () => () => undefined, dispose() {}});
  return {parent, child, stop, setWriteLocked, get: () => live,
    type: (text: string) => {if (!locked) live = createNoteDocumentFromText(child.page.id, text, () => noteBlockId("child-body"));}};
}

it("freezes an open descendant before async preparation, through commit, and also freezes newly mounted editors", async () => {
  const f = await fixture(), preparing = deferred(), transaction = deferred(), expected = f.get();
  const prepare = vi.fn(async () => {
    expect(isTextEditorHeld(f.child.page.id)).toBe(true); expect(isTextEditorSaveBlocked(f.child.page.id)).toBe(false);
    await preparing.promise;
    const current = await gateway.loadDocument(f.child.page.id);
    await gateway.saveDocument({document: f.get(), expectedRevision: current.page.revision, saveReason: "manual"});
    return true;
  });
  const stop = getReaderCloseCoordinator().register("guard-descendant", {prepare, isActive: () => false, commit() {}});
  const apply = vi.fn<typeof gateway.applyMetadataBatch>(async request => {await transaction.promise; return gateway.applyMetadataBatch(request);});
  let stopNew = () => {};
  try {
    const deleting = applyNotePageStatus(new NotesClient({...gateway, applyMetadataBatch: apply}), f.parent.page.id, "trashed");
    expect(prepare).toHaveBeenCalledOnce(); f.type("input during prepare"); expect(f.get()).toEqual(expected);
    preparing.resolve(); await vi.waitFor(() => expect(apply).toHaveBeenCalledOnce());
    f.type("input while committing"); expect(f.get()).toEqual(expected);
    const setNewLocked = vi.fn();
    stopNew = registerTextInstallPort("new-during-metadata", {read: () => expected, setWriteLocked: setNewLocked,
      isInstalling: false, prepare: () => () => undefined, prepareStructure: () => () => undefined, dispose() {}});
    expect(setNewLocked).toHaveBeenLastCalledWith(true);
    transaction.resolve(); await deleting;
    const saved = await gateway.loadDocument(f.child.page.id);
    expect(saved.page.status).toBe("trashed"); expect(saved.document).toEqual({...expected, title: "child"});
    expect(isTextEditorHeld(f.child.page.id)).toBe(false); expect(isTextEditorHeld("new-during-metadata")).toBe(false);
    expect(f.setWriteLocked).toHaveBeenLastCalledWith(false); expect(setNewLocked).toHaveBeenLastCalledWith(false);
  } finally {preparing.resolve(); transaction.resolve(); stopNew(); stop(); f.stop();}
});

it("releases all descendant input after a failed transaction and retains the prepared body", async () => {
  const f = await fixture(), transaction = deferred(), expected = f.get();
  const stop = getReaderCloseCoordinator().register("guard-failure", {prepare: async () => {
    const current = await gateway.loadDocument(f.child.page.id);
    await gateway.saveDocument({document: f.get(), expectedRevision: current.page.revision, saveReason: "manual"}); return true;
  }, isActive: () => false, commit() {}});
  const apply = vi.fn<typeof gateway.applyMetadataBatch>(async () => {await transaction.promise; throw new NotesGatewayError("storage_error", "commit rejected");});
  try {
    const deleting = applyNotePageStatus(new NotesClient({...gateway, applyMetadataBatch: apply}), f.parent.page.id, "trashed");
    await vi.waitFor(() => expect(apply).toHaveBeenCalledOnce());
    f.type("blocked while committing"); expect(f.get()).toEqual(expected);
    const rejected = expect(deleting).rejects.toMatchObject({code: "storage_error"}); transaction.resolve(); await rejected;
    expect(isTextEditorHeld(f.child.page.id)).toBe(false);
    const saved = await gateway.loadDocument(f.child.page.id);
    expect(saved.page.status).toBe("active"); expect(saved.document).toEqual({...expected, title: "child"});
    f.type("can continue editing"); expect(f.get()).not.toEqual(expected);
  } finally {transaction.resolve(); stop(); f.stop();}
});

it("rejects composition or asset import before locking or starting a delete", async () => {
  const f = await fixture(), apply = vi.fn(gateway.applyMetadataBatch);
  const stop = registerTextInstallPort("composing", {read: f.get, setWriteLocked: vi.fn(), isInstalling: false,
    prepare: () => () => undefined, prepareStructure: () => () => undefined, dispose() {}}, () => false);
  try {
    await expect(applyNotePageStatus(new NotesClient({...gateway, applyMetadataBatch: apply}), f.parent.page.id, "trashed")).rejects.toThrow("正在输入");
    expect(apply).not.toHaveBeenCalled(); expect(f.setWriteLocked).not.toHaveBeenCalled();
    expect(isTextEditorHeld(f.child.page.id)).toBe(false);
  } finally {stop(); f.stop();}
});
