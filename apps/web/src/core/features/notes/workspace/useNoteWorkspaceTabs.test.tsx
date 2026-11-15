// @vitest-environment happy-dom
import { act, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {selectAppStorage} from "../../../runtime/appStorage";
import {loadNoteTabs} from "./noteTabSession";
import { notePageId } from "@research-assistant/core/notes";
import { WorkspaceTabBar, WorkspaceTabsProvider } from "../../../components/workspace-tabs/WorkspaceTabs";
import { useNoteWorkspaceTabs } from "./useNoteWorkspaceTabs";

vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
let root: ReturnType<typeof createRoot> | undefined, host: HTMLDivElement;
beforeEach(()=>{localStorage.clear();selectAppStorage(localStorage);});
afterEach(async () => { if (root) await act(async () => root!.unmount()); host?.remove(); });
async function harness() {
  const prepare = vi.fn().mockResolvedValue(true), open = vi.fn(), closed = vi.fn(), cleared = vi.fn();
  let page = notePageId("a");
  let api!: { select: (id: string, pinned?: boolean) => void; setActive: (active: boolean) => void; dirty: () => void; showIndex: () => void };
  function Body() {
    const [selected, setSelected] = useState<typeof page | undefined>(page), [active, setActive] = useState(true), [dirty, setDirty] = useState(false);
    const [homeActive, setHomeActive] = useState(false);
    const current = useRef(selected); current.current = selected;
    const tabs = useNoteWorkspaceTabs({ page: selected ? { id: selected, title: selected } : undefined,
      enabled: true, active, homeActive, dirty, prepare, getPageId: () => current.current,
      open: async (id, background) => { open(id, background); page = id; current.current = id; setSelected(id); },
      clear: preserveHome => { cleared(preserveHome); current.current = undefined; setSelected(undefined); }, onClose: closed });
    api = { select: (id, pinned) => { if (pinned) tabs.pin(id); page = notePageId(id); setDirty(false); setSelected(page); }, setActive, dirty: () => setDirty(true), showIndex: () => setHomeActive(true) };
    return <WorkspaceTabBar owner={active ? "notes" : "library"} textActive={false} navigate={(_, __, ready) => { ready(); return true; }} />;
  }
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root!.render(<WorkspaceTabsProvider><Body /></WorkspaceTabsProvider>));
  return { get api() { return api; }, prepare, open, closed, cleared };
}
it("replaces previews, fixes on edit or double click, and preserves fixed notes", async () => {
  const h = await harness();
  expect(host.querySelector('[data-preview="true"]')?.textContent).toBe("a");
  await act(async () => h.api.select("b"));
  expect(host.querySelectorAll('[aria-label^="关闭 "]')).toHaveLength(1);
  await act(async () => h.api.dirty());
  expect(host.querySelector('[data-preview="true"]')).toBeNull();
  await act(async () => h.api.select("c"));
  expect(host.querySelectorAll('[aria-label^="关闭 "]')).toHaveLength(2);
  await act(async () => host.querySelector<HTMLButtonElement>('[data-preview="true"] > button[role="tab"]')!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
  expect(host.querySelector('[data-preview="true"]')).toBeNull();
});
it("reopens the retained tab identities after unmount and preserves explicitly closed tabs",async()=>{
  const first=await harness();
  await act(async()=>first.api.select("a",true));
  await act(async()=>first.api.select("b",true));
  await act(async()=>root!.unmount());host.remove();
  const second=await harness();
  expect(host.querySelectorAll('[aria-label^="关闭 "]')).toHaveLength(2);
  expect(second.open).not.toHaveBeenCalled();
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="关闭 b"]')!.click());
  expect(loadNoteTabs().map(tab=>tab.id)).toEqual(["a"]);
});

it("keeps the full index active when retained document tabs are closed in the background", async () => {
  const h = await harness();
  await act(async () => h.api.select("a", true));
  await act(async () => h.api.select("b", true));
  await act(async () => h.api.showIndex());
  expect(host.querySelector('[aria-selected="true"]')?.getAttribute("aria-label")).toBe("返回笔记首页");
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="关闭 b"]')!.click());
  expect(h.open).toHaveBeenCalledWith("a", true);
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="关闭 a"]')!.click());
  expect(h.cleared).toHaveBeenCalledExactlyOnceWith(true);
  expect(host.querySelector('[aria-selected="true"]')?.getAttribute("aria-label")).toBe("返回笔记首页");
});
it("does not close when saving is blocked and chooses a sibling without navigating a hidden workspace", async () => {
  const h = await harness();
  await act(async () => h.api.select("a", true));
  await act(async () => h.api.select("b", true));
  h.prepare.mockResolvedValueOnce(false);
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="关闭 b"]')!.click());
  expect(host.querySelectorAll('[aria-label^="关闭 "]')).toHaveLength(2); expect(h.closed).not.toHaveBeenCalled();
  await act(async () => h.api.setActive(false));
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="关闭 b"]')!.click());
  expect(h.open).toHaveBeenCalledWith("a", true);
  expect(host.querySelectorAll('[aria-label^="关闭 "]')).toHaveLength(1);
  expect(h.closed).toHaveBeenCalledWith(["a"], "a", undefined);
});
