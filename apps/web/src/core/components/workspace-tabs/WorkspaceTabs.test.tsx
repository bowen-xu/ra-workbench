// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { WorkspaceTabBar, WorkspaceTabsProvider, usePublishWorkspaceTabs, type WorkspaceTab } from "./WorkspaceTabs";
import type { WorkspaceView } from "../../useWorkspaceNavigation";

vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
let root: ReturnType<typeof createRoot> | undefined;
let host: HTMLDivElement | undefined;
afterEach(async () => { if (root) await act(async () => root!.unmount()); host?.remove(); });
function Group({ id, tabs, enabled = true }: { readonly id: string; readonly tabs: readonly WorkspaceTab[]; readonly enabled?: boolean }) {
  usePublishWorkspaceTabs(id, tabs, enabled); return null;
}
it("retains the same tab elements across sections and routes selection through the shell guard", async () => {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const select = vi.fn(), pin = vi.fn(), close = vi.fn();
  const navigate = vi.fn((_: WorkspaceView, _destination: string | undefined, ready: () => void) => { ready(); return true; });
  const note: WorkspaceTab = { id: "notes:n", owner: "notes", title: "Note", destination: "n", presentation: "document", active: true, preview: true, select, pin, close };
  function Shell({ owner }: { readonly owner: WorkspaceView }) {
    return <WorkspaceTabsProvider><Group id="library" tabs={[{ id: "pdf:p", owner: "library", title: "Paper", active: true, select }]} /><Group id="notes" tabs={[note]} /><WorkspaceTabBar owner={owner} textActive={false} navigate={navigate} /></WorkspaceTabsProvider>;
  }
  await act(async () => root!.render(<Shell owner="library" />));
  const noteButton = host.querySelector<HTMLButtonElement>('[data-tab-id="notes:n"] > button[role="tab"]')!;
  expect(host.querySelector('[aria-selected="true"]')?.textContent).toContain("Paper");
  await act(async () => root!.render(<Shell owner="notes" />));
  expect(host.querySelector('[data-tab-id="notes:n"] > button[role="tab"]')).toBe(noteButton);
  expect(noteButton.getAttribute("aria-selected")).toBe("true");
  await act(async () => noteButton.click()); expect(navigate).toHaveBeenCalledWith("notes", "n", expect.any(Function), "document");
  expect(select).toHaveBeenCalledOnce();
  await act(async () => noteButton.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }))); expect(pin).toHaveBeenCalledOnce();
  await act(async () => host!.querySelector<HTMLButtonElement>('[aria-label="关闭 Note"]')!.click()); expect(close).toHaveBeenCalledOnce();
  navigate.mockImplementation(() => false); select.mockClear();
  await act(async () => noteButton.click()); expect(select).not.toHaveBeenCalled();
});
it("an embedded disabled publisher cannot unregister the main notes group", async () => {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const tab: WorkspaceTab = { id: "notes:a", owner: "notes", title: "Kept note", active: true, select: vi.fn() };
  function Shell({ embedded }: { readonly embedded: boolean }) {
    return <WorkspaceTabsProvider><Group id="notes" tabs={[tab]} />{embedded && <Group id="notes" tabs={[]} enabled={false} />}<WorkspaceTabBar owner="notes" textActive={false} navigate={(_, __, ready) => { ready(); return true; }} /></WorkspaceTabsProvider>;
  }
  await act(async () => root!.render(<Shell embedded />));
  await act(async () => root!.render(<Shell embedded={false} />));
  expect(host.textContent).toContain("Kept note");
});
it("appends newly opened files at the global end even when their publisher precedes another workspace", async () => {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const select = vi.fn();
  const tab = (id: string, owner: WorkspaceView): WorkspaceTab => ({ id, owner, title: id, active: false, select });
  function Shell({ extra, replace = false }: { extra: boolean; replace?: boolean }) { return <WorkspaceTabsProvider>
    <Group id="writing" tabs={[tab("a", "writing"), ...(extra ? [tab(replace ? "next" : "preview", "writing")] : [])]} />
    <Group id="notes" tabs={[tab("note", "notes")]} />
    <WorkspaceTabBar owner="writing" textActive={false} navigate={(_, __, ready) => { ready(); return true; }} /></WorkspaceTabsProvider>; }
  const order = () => [...host!.querySelectorAll("[data-tab-id]")].map(element => element.getAttribute("data-tab-id"));
  await act(async () => root!.render(<Shell extra={false} />)); expect(order()).toEqual(["a", "note"]);
  await act(async () => root!.render(<Shell extra />)); expect(order()).toEqual(["a", "note", "preview"]);
  await act(async () => root!.render(<Shell extra replace />)); expect(order()).toEqual(["a", "note", "next"]);
});
it("keeps one context home and renders PDF, code and pending state without a diamond",async()=>{
  host=document.createElement("div");document.body.append(host);root=createRoot(host);
  const select=vi.fn();
  function Shell({owner}:{owner:WorkspaceView}){return <WorkspaceTabsProvider><Group id="library" tabs={[{id:"library:home",owner:"library",title:"返回文献列表",home:true,active:false,select},{id:"paper",owner:"library",title:"Paper",icon:"pdf",active:false,select}]} /><Group id="notes" tabs={[{id:"notes:home",owner:"notes",title:"返回笔记首页",home:true,active:true,select}]} /><Group id="writing" tabs={[{id:"writing:home",owner:"writing",title:"返回写作项目列表",home:true,active:false,select},{id:"code",owner:"writing",title:"main.tex",icon:"code",active:true,pending:true,select}]} /><WorkspaceTabBar owner={owner} textActive={false} navigate={(_,__,ready)=>{ready();return true;}} /></WorkspaceTabsProvider>;}
  await act(async()=>root!.render(<Shell owner="notes"/>));
  expect(host.querySelectorAll('[data-home="true"]')).toHaveLength(1);
  expect(host.querySelector('[data-home="true"] button')?.getAttribute("aria-label")).toBe("返回笔记首页");
  expect(host.querySelector(".workspace-tab-pdf")?.textContent).toBe("PDF");
  expect(host.querySelector('[data-tab-id="code"] .lucide-file-code-corner')).not.toBeNull();
  expect(host.querySelector('[aria-label="待审变更"]')?.textContent).toBe("");
  await act(async()=>root!.render(<Shell owner="writing"/>));
  expect(host.querySelector('[data-home="true"] button')?.getAttribute("aria-label")).toBe("返回写作项目列表");
});

it("uses the research-context home while a document's original storage owner keeps the active tab", async () => {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const select = vi.fn();
  await act(async () => root!.render(<WorkspaceTabsProvider>
    <Group id="library" tabs={[{id:"library:home", owner:"library", title:"返回文献列表", home:true, active:false, select}]} />
    <Group id="notes" tabs={[{id:"notes:home", owner:"notes", title:"返回笔记首页", home:true, active:false, select},
      {id:"notes:page", owner:"notes", title:"阅读批注", active:true, presentation:"document", select}]} />
    <WorkspaceTabBar owner="notes" homeOwner="library" textActive={false} navigate={(_owner, _destination, ready) => {ready(); return true;}} />
  </WorkspaceTabsProvider>));
  expect(host.querySelector('[data-home="true"] button')?.getAttribute('aria-label')).toBe("返回文献列表");
  expect(host.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toBe("阅读批注");
});
