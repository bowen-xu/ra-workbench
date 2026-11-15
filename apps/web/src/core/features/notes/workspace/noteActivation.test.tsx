// @vitest-environment happy-dom
import {act, StrictMode, useEffect} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {noteBlockId, notePageId} from "@research-assistant/core/notes";
import type {NoteDocumentSnapshot, NotesGateway} from "@research-assistant/data/notes";
import {NotesGatewayError} from "@research-assistant/data/notes";
import type {AppHbsDocumentGateway} from "../../../services/hbsDocumentGateway";
import {NotesClient} from "../NotesClient";
import {useNoteDocumentSession} from "./useNoteDocumentSession";
import {useNoteWorkspaceTabs} from "./useNoteWorkspaceTabs";

vi.mock("./noteRecovery", () => ({loadNoteRecoveryDraft: () => undefined, clearNoteRecoveryDraft: vi.fn()}));
const cleanups: (() => Promise<void>)[] = [];
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
afterEach(async () => {for (const cleanup of cleanups.splice(0)) await cleanup();});
function snapshot(id: string): NoteDocumentSnapshot {
  return {page: {id: notePageId(id), title: id, schemaVersion: 1, revision: 3, position: 0, status: "active", isFavorite: false,
    projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now"}, document: {schemaVersion: 1, pageId: notePageId(id), blocks: [{
      id: noteBlockId(`block-${id}`), type: "paragraph", props: {}, children: [], content: {kind: "inline", items: [{type: "text", text: id, styles: []}]},
    }]}};
}
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {resolve = yes; reject = no;});
  return {promise, resolve, reject};
}
async function fixture() {
  const gateway = {loadDocument: vi.fn(async (id: string) => snapshot(id)), listOutgoing: vi.fn(async () => []), listBacklinks: vi.fn(async () => [])};
  const hbs: AppHbsDocumentGateway = {
    read: vi.fn(async (_kind, id) => ({version: 3, saved: snapshot(id).document, baseline: snapshot(id).document})),
    commit: vi.fn(), readPreservation: vi.fn(async () => null), savePreservation: vi.fn(async () => ({revision: 1})),
  };
  const options = {client: new NotesClient(gateway as unknown as NotesGateway), hbsGateway: hbs, onError: vi.fn(), onNotice: vi.fn(), onLoaded: vi.fn()};
  let current!: ReturnType<typeof useNoteDocumentSession>, tabs!: ReturnType<typeof useNoteWorkspaceTabs>;
  function Harness() {
    current = useNoteDocumentSession(options);
    const page = current.loadState.phase === "loading" || current.loadState.phase === "failed"
      ? {id: current.loadState.pageId, title: current.loadState.pageId} : current.snapshot?.page;
    tabs = useNoteWorkspaceTabs({page, enabled: true, active: true, dirty: page?.id === current.snapshot?.page.id && current.saveStatus !== "saved",
      open: id => current.loadPage(id), getPageId: current.getSelectedPageId,
      prepare: id => current.ensureDurable(false, id), clear: current.deactivate, dispose: current.disposeSession});
    const retained = tabs.retainedIds.join("|");
    useEffect(() => current.retain(tabs.retainedIds), [retained, current.snapshot?.page.id, current.retain]);
    return null;
  }
  const host = document.createElement("div"), root = createRoot(host);
  await act(async () => root.render(<StrictMode><Harness/></StrictMode>));
  cleanups.push(async () => {await act(async () => root.unmount()); host.remove();});
  return {gateway, hbs, options, get: () => current, tabs: () => tabs.retainedIds, pin: (id: string) => tabs.pin(id)};
}

it.each([false, true])("selects the target tab while retaining the previous owner during initialization (pinned=%s)", async pinned => {
  const f = await fixture(), pending = deferred<unknown>();
  await act(async () => f.get().loadPage(notePageId("a")));
  const original = f.get().saveControllerRef.current!, editorId = f.get().editorSessionId;
  const disposed = vi.spyOn(original, "dispose");
  if (pinned) await act(async () => f.pin("a"));
  vi.mocked(f.hbs.read).mockImplementationOnce(() => pending.promise);
  let opening!: Promise<void>;
  await act(async () => {opening = f.get().loadPage(notePageId("b"));});
  expect(f.tabs()).toContain("b");
  expect(f.get().loadState).toMatchObject({phase: "loading", pageId: "b"});
  const during = {page: f.get().snapshot?.page.id, owner: f.get().saveControllerRef.current, editorId: f.get().editorSessionId, disposed: disposed.mock.calls.length};
  await act(async () => {pending.resolve({version: 3, saved: snapshot("b").document, baseline: snapshot("b").document}); await opening;});
  expect(during.page).toBe("a"); expect(during.owner).toBe(original);
  expect(during.editorId).toBe(editorId); expect(during.disposed).toBe(0);
  expect(f.get().snapshot?.page.id).toBe("b");
  expect(f.get().loadState).toEqual({phase: "ready", pageId: "b"});
  const installed = f.get().retainedSessions.find(session => session.snapshot.page.id === "b")!;
  expect(installed).toBeDefined(); expect(f.get().saveControllerRef.current).toBe(installed.controller);
  expect(f.get().editorSessionId).toBe(installed.editorSessionId);
  expect(disposed).toHaveBeenCalledTimes(pinned ? 0 : 1);
});

it("retains the previous owner and allows retry when the target cannot open", async () => {
  const f = await fixture(); await act(async () => f.get().loadPage(notePageId("a")));
  const owner = f.get().saveControllerRef.current, editor = f.get().editorSessionId;
  vi.mocked(f.hbs.read).mockRejectedValueOnce(new Error("synthetic read failure"));
  await act(async () => {await expect(f.get().loadPage(notePageId("b"))).rejects.toThrow("synthetic read failure");});
  expect(f.get().snapshot?.page.id).toBe("a"); expect(f.get().saveControllerRef.current).toBe(owner);
  expect(f.get().editorSessionId).toBe(editor); expect(f.get().loadState).toMatchObject({phase: "failed", pageId: "b"});
  await act(async () => f.get().loadPage(notePageId("b")));
  expect(f.get().snapshot?.page.id).toBe("b"); expect(f.get().loadState.phase).toBe("ready");
});

it("reports a missing page only when the document read returns not_found, and clears that state on retry", async () => {
  const f = await fixture(); await act(async () => f.get().loadPage(notePageId("a")));
  const owner = f.get().saveControllerRef.current;
  f.gateway.loadDocument.mockRejectedValueOnce(new NotesGatewayError("not_found", "operation=open-7 · page missing"));
  await act(async () => { await expect(f.get().loadPage(notePageId("b"))).rejects.toThrow("page missing"); });
  expect(f.get().loadState).toMatchObject({ phase: "failed", pageId: "b", failure: {
    kind: "missing", code: "not_found", message: "operation=open-7 · page missing",
  }});
  expect(f.get().saveControllerRef.current).toBe(owner);
  await act(async () => f.get().loadPage(notePageId("b")));
  expect(f.get().loadState).toEqual({phase: "ready", pageId: "b"});
});

it("does not label missing links or owner initialization errors as a missing note", async () => {
  const f = await fixture();
  f.gateway.listOutgoing.mockRejectedValueOnce(new NotesGatewayError("not_found", "link index missing"));
  await act(async () => { await expect(f.get().loadPage(notePageId("b"))).rejects.toThrow("link index missing"); });
  expect(f.get().loadState).toMatchObject({phase: "failed", failure: {kind: "failed", code: "not_found"}});
  vi.mocked(f.hbs.read).mockRejectedValueOnce(new NotesGatewayError("not_found", "owner missing"));
  await act(async () => { await expect(f.get().loadPage(notePageId("b"))).rejects.toThrow("owner missing"); });
  expect(f.get().loadState).toMatchObject({phase: "failed", failure: {kind: "failed", message: "owner missing"}});
});

it("ignores an older owner initialization after a newer page has activated", async () => {
  const f = await fixture(), pending = deferred<unknown>();
  await act(async () => f.get().loadPage(notePageId("a")));
  vi.mocked(f.hbs.read).mockImplementationOnce(() => pending.promise);
  let opening!: Promise<void>;
  await act(async () => {opening = f.get().loadPage(notePageId("b"));});
  await act(async () => f.get().loadPage(notePageId("c")));
  const owner = f.get().saveControllerRef.current;
  await act(async () => {pending.resolve({version: 3, saved: snapshot("b").document, baseline: snapshot("b").document}); await opening;});
  expect(f.get().snapshot?.page.id).toBe("c"); expect(f.get().saveControllerRef.current).toBe(owner);
  expect(f.get().retainedSessions.some(session => session.snapshot.page.id === "b")).toBe(false);
  expect(f.get().loadState).toEqual({phase: "ready", pageId: "c"});
});

it("does not publish a late editor after navigation returns Home", async () => {
  const f = await fixture(), pending = deferred<unknown>();
  await act(async () => f.get().loadPage(notePageId("a")));
  vi.mocked(f.hbs.read).mockImplementationOnce(() => pending.promise);
  let opening!: Promise<void>;
  await act(async () => {opening = f.get().loadPage(notePageId("b"));});
  await act(async () => f.get().deactivate());
  await act(async () => {pending.resolve({version: 3, saved: snapshot("b").document, baseline: snapshot("b").document}); await opening;});
  expect(f.get().snapshot).toBeUndefined(); expect(f.get().saveControllerRef.current).toBeUndefined();
  expect(f.get().loadState.phase).toBe("idle");
  expect(f.get().retainedSessions.some(session => session.snapshot.page.id === "b")).toBe(false);
});

it("marks initial owner failures as retryable instead of leaving an empty loading page", async () => {
  const f = await fixture();
  vi.mocked(f.hbs.read).mockRejectedValueOnce(new Error("synthetic initial read failure"));
  await act(async () => {await expect(f.get().loadPage(notePageId("a"))).rejects.toThrow("synthetic initial read failure");});
  expect(f.get().snapshot).toBeUndefined(); expect(f.get().loadState).toMatchObject({phase: "failed", pageId: "a"});
  await act(async () => f.get().loadPage(notePageId("a")));
  expect(f.get().snapshot?.page.id).toBe("a"); expect(f.get().retainedSessions).toHaveLength(1);
});

it("does not cancel the target load when a different editor is disposed", async () => {
  const f = await fixture(), pending = deferred<unknown>();
  await act(async () => f.get().loadPage(notePageId("a")));
  vi.mocked(f.hbs.read).mockImplementationOnce(() => pending.promise);
  let opening!: Promise<void>;
  await act(async () => {opening = f.get().loadPage(notePageId("b"));});
  await act(async () => f.get().disposeSession(notePageId("a")));
  await act(async () => {pending.resolve({version: 3, saved: snapshot("b").document, baseline: snapshot("b").document}); await opening;});
  expect(f.get().snapshot?.page.id).toBe("b"); expect(f.get().loadState.phase).toBe("ready");
  expect(f.get().saveControllerRef.current).toBeDefined();
});

it("cancels a closed target even before it has an editor, and permits a fresh reopen", async () => {
  const f = await fixture(), pending = deferred<unknown>();
  await act(async () => f.get().loadPage(notePageId("a")));
  vi.mocked(f.hbs.read).mockImplementationOnce(() => pending.promise);
  let opening!: Promise<void>;
  await act(async () => {opening = f.get().loadPage(notePageId("b"));});
  await act(async () => f.get().disposeSession(notePageId("b")));
  await act(async () => f.get().loadPage(notePageId("b")));
  const owner = f.get().saveControllerRef.current;
  await act(async () => {pending.resolve({version: 3, saved: snapshot("b").document, baseline: snapshot("b").document}); await opening;});
  expect(f.get().snapshot?.page.id).toBe("b"); expect(f.get().saveControllerRef.current).toBe(owner);
  expect(f.get().loadState.phase).toBe("ready");
});

it("selects a named target before its document read finishes", async () => {
  const f = await fixture(), pending = deferred<NoteDocumentSnapshot>();
  await act(async () => f.get().loadPage(notePageId("a")));
  f.gateway.loadDocument.mockReturnValueOnce(pending.promise);
  let opening!: Promise<void>;
  await act(async () => {opening = f.get().loadPage(notePageId("b"), true, undefined, false, "Target B");});
  expect(f.get().loadState).toEqual({phase: "loading", pageId: "b", title: "Target B"});
  expect(f.get().getSelectedPageId()).toBe("b"); expect(f.get().snapshot?.page.id).toBe("a");
  await act(async () => {pending.resolve(snapshot("b")); await opening;});
  expect(f.get().snapshot?.page.id).toBe("b");
});

it("reactivates a cached editor without reading its document or owner again", async () => {
  const f = await fixture();
  await act(async () => f.get().loadPage(notePageId("a")));
  await act(async () => f.pin("a"));
  const owner = f.get().saveControllerRef.current, editor = f.get().editorSessionId;
  await act(async () => f.get().loadPage(notePageId("b")));
  const reads = f.gateway.loadDocument.mock.calls.length, ownerReads = vi.mocked(f.hbs.read).mock.calls.length;
  await act(async () => f.get().loadPage(notePageId("a")));
  expect(f.get().loadState.phase).toBe("ready"); expect(f.get().saveControllerRef.current).toBe(owner);
  expect(f.get().editorSessionId).toBe(editor); expect(f.gateway.loadDocument).toHaveBeenCalledTimes(reads);
  expect(f.hbs.read).toHaveBeenCalledTimes(ownerReads);
});
