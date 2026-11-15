// @vitest-environment happy-dom
import {act, useRef, useState} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {notePageId, type NoteLifecycleStatus, type NoteMetadataAction} from "@research-assistant/core/notes";
import {WorkspaceTabBar, WorkspaceTabsProvider} from "../../../components/workspace-tabs/WorkspaceTabs";
import {NOTE_METADATA_CHANGED, type NoteMetadataChanged} from "../../../../../../../plugins/note_manager/src/features/notes/workspace/noteFavoriteActions";
import {useNoteWorkspaceTabs} from "./useNoteWorkspaceTabs";

vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
let root: ReturnType<typeof createRoot> | undefined, host: HTMLDivElement;
afterEach(async () => {if (root) await act(async () => root!.unmount()); host?.remove();});

async function harness(initialStatus: NoteLifecycleStatus = "active", beforeOpen: (id: string) => Promise<void> = async () => {}) {
  const prepare = vi.fn(async () => true), open = vi.fn(), closed = vi.fn(), clear = vi.fn(), dispose = vi.fn();
  let api!: {select: (id: string) => void; hide: () => void; index: () => void; selected: string | undefined};
  function Body() {
    const [selected, setSelected] = useState<string | undefined>("a"), [active, setActive] = useState(true), [homeActive, setHomeActive] = useState(false);
    const current = useRef(selected); current.current = selected;
    const generation = useRef(0);
    const page = selected ? {id: notePageId(selected), title: selected, status: initialStatus} : undefined;
    const tabs = useNoteWorkspaceTabs({page,
      active, enabled: true, dirty: false, homeActive, prepare, dispose,
      getPageId: () => current.current === undefined ? undefined : notePageId(current.current),
      open: async (id, background) => {open(id, background); const request = generation.current; await beforeOpen(id);
        if (request === generation.current) {current.current = id; setSelected(id);}},
      clear: preserveHome => {clear(preserveHome); generation.current++; current.current = undefined; setSelected(undefined);}, onClose: closed});
    api = {select: id => {tabs.pin(id); current.current = id; setSelected(id);}, hide: () => setActive(false), index: () => setHomeActive(true), selected};
    return <WorkspaceTabBar owner={active ? "notes" : "library"} textActive={false} navigate={(_, __, ready) => {ready(); return true;}}/>;
  }
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root!.render(<WorkspaceTabsProvider><Body/></WorkspaceTabsProvider>));
  return {get api() {return api;}, prepare, open, closed, clear, dispose};
}
async function publish(ids: readonly string[], status: NoteLifecycleStatus = "trashed", action: NoteMetadataAction = status === "trashed" ? "trash" : status === "active" ? "restore" : "archive") {
  const pages = ids.map(id => ({id: notePageId(id), title: id, status, revision: 1, schemaVersion: 1 as const,
    position: 0, isFavorite: false, tagIds: [], projectIds: [], createdAt: "now", updatedAt: "now"}));
  await act(async () => window.dispatchEvent(new CustomEvent<NoteMetadataChanged>(NOTE_METADATA_CHANGED, {detail: {pages, bodyChangedIds: [], action}})));
}
const tabIds = () => [...host.querySelectorAll<HTMLElement>("[data-tab-id]:not([data-home])")].map(tab => tab.dataset.tabId);

it("closes a successfully deleted active note without saving the deleted session again", async () => {
  const h = await harness();
  await publish(["a"]);
  expect(tabIds()).toEqual([]); expect(h.api.selected).toBeUndefined();
  expect(h.prepare).not.toHaveBeenCalled(); expect(h.dispose).toHaveBeenCalledExactlyOnceWith("a");
  expect(h.clear).toHaveBeenCalledExactlyOnceWith(false);
  expect(h.closed).toHaveBeenCalledExactlyOnceWith([], undefined, "workspace");
});

it("closes a full cascade in one pass and selects an unaffected sibling", async () => {
  const h = await harness();
  await act(async () => h.api.select("a"));
  await act(async () => h.api.select("b"));
  await act(async () => h.api.select("c"));
  await publish(["b", "c"]);
  expect(tabIds()).toEqual(["notes:a"]); expect(h.api.selected).toBe("a");
  expect(h.open).toHaveBeenCalledExactlyOnceWith("a", false);
  expect(h.dispose.mock.calls).toEqual([["b"], ["c"]]);
  expect(h.closed).toHaveBeenCalledExactlyOnceWith(["a"], "a", "workspace");
});

it("closes deleted background tabs without switching the active note", async () => {
  const h = await harness();
  await act(async () => h.api.select("a"));
  await act(async () => h.api.select("b"));
  await publish(["a"]);
  expect(tabIds()).toEqual(["notes:b"]); expect(h.api.selected).toBe("b");
  expect(h.open).not.toHaveBeenCalled(); expect(h.clear).not.toHaveBeenCalled();
  expect(h.closed).toHaveBeenCalledExactlyOnceWith(["b"], "b", "workspace");
});

it("keeps a hidden workspace hidden and the full index selected when deleting its retained note", async () => {
  const h = await harness();
  await act(async () => h.api.select("a"));
  await act(async () => h.api.select("b"));
  await act(async () => {h.api.hide(); h.api.index();});
  await publish(["b"]);
  expect(h.open).toHaveBeenCalledExactlyOnceWith("a", true);
  expect(h.clear).toHaveBeenCalledExactlyOnceWith(true);
  expect(h.closed).toHaveBeenCalledExactlyOnceWith(["a"], "a", undefined);
});

it("allows an intentionally reopened trash note and does not close on archive, restore or failed batches", async () => {
  const h = await harness("trashed");
  expect(tabIds()).toEqual(["notes:a"]);
  await publish([], "trashed"); await publish(["a"], "archived"); await publish(["a"], "active");
  expect(tabIds()).toEqual(["notes:a"]); expect(h.closed).not.toHaveBeenCalled();
  await publish(["a"]); expect(tabIds()).toEqual([]);
  await act(async () => h.api.select("a"));
  expect(tabIds()).toEqual(["notes:a"]); expect(h.api.selected).toBe("a");
});

it("keeps an already trashed parent open when another lifecycle operation only updates its body", async () => {
  const h = await harness("trashed");
  await publish(["a"], "trashed", "restore");
  await publish(["a"], "trashed", "trash");
  expect(tabIds()).toEqual(["notes:a"]); expect(h.api.selected).toBe("a");
  expect(h.closed).not.toHaveBeenCalled(); expect(h.dispose).not.toHaveBeenCalled();
});

it("cancels a sibling load if a later successful deletion removes that destination", async () => {
  let release!: () => void;
  const pending = new Promise<void>(resolve => {release = resolve;});
  const h = await harness("active", id => id === "c" ? pending : Promise.resolve());
  await act(async () => h.api.select("a"));
  await act(async () => h.api.select("b"));
  await act(async () => h.api.select("c"));
  await act(async () => h.api.select("b"));
  await publish(["b"]);
  expect(tabIds()).toEqual(["notes:a", "notes:c"]);
  await publish(["c"]);
  expect(tabIds()).toEqual(["notes:a"]); expect(h.api.selected).toBe("a");
  await act(async () => release());
  expect(h.api.selected).toBe("a");
  expect(h.closed).toHaveBeenCalledExactlyOnceWith(["a"], "a", "workspace");
});

it("does not reconcile an unrelated background deletion ahead of an in-flight sibling selection", async () => {
  let release!: () => void;
  const pending = new Promise<void>(resolve => {release = resolve;});
  const h = await harness("active", id => id === "c" ? pending : Promise.resolve());
  await act(async () => h.api.select("a"));
  await act(async () => h.api.select("b"));
  await act(async () => h.api.select("c"));
  await act(async () => h.api.select("b"));
  await publish(["b"]); await publish(["a"]);
  expect(h.closed).not.toHaveBeenCalled();
  await act(async () => release());
  expect(tabIds()).toEqual(["notes:c"]); expect(h.api.selected).toBe("c");
  expect(h.closed).toHaveBeenCalledExactlyOnceWith(["c"], "c", "workspace");
});
