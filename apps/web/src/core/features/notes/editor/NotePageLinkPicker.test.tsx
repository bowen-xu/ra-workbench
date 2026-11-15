// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {notePageId,type NoteIndexItem} from "@research-assistant/core/notes";
import type {NoteTargetCatalogPage} from "@research-assistant/data/notes";
import {NotePageLinkPicker} from "./NotePageLinkPicker";
vi.mock("../../../components/overlay/OverlayDialog",()=>({Dialog:({children}: {children:React.ReactNode})=><div>{children}</div>}));
vi.mock("../NotesClient",()=>({useNotesClient:()=>({}),useNotesVersion:()=>0}));
vi.mock("../workspace/useNoteCatalogTargets",()=>({useNoteCatalogTargets:()=>undefined}));
let root:ReturnType<typeof createRoot>,host:HTMLDivElement;
const query=vi.fn<(...args:[string,string?])=>Promise<NoteTargetCatalogPage>>(),select=vi.fn();
const item=(id:string):NoteIndexItem=>({id:`page:${id}`,title:id,snippet:"",category:"independent",status:"active",isFavorite:false,updatedAt:"2026-10-24",target:{kind:"page",pageId:notePageId(id)}});
beforeEach(()=>{vi.useFakeTimers();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);query.mockReset();select.mockReset();host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.useRealTimers();vi.unstubAllGlobals();});
const mount=async()=>{await act(async()=>root.render(<NotePageLinkPicker pages={[]} queryTargets={query} excludePageId="self" onSelect={select} onClose={()=>{}}/>));await act(async()=>vi.advanceTimersByTimeAsync(0));};
it("queries one candidate page on demand and appends only an explicit continuation",async()=>{
  query.mockResolvedValueOnce({items:[item("self"),item("one")],nextCursor:"next"}).mockResolvedValueOnce({items:[item("two")]});
  await mount();expect(query.mock.calls).toEqual([["",undefined]]);expect(host.textContent).not.toContain("self");
  await act(async()=>Array.from(host.querySelectorAll("button")).find(button=>button.textContent==="加载更多笔记")!.click());
  expect(query.mock.calls[1]).toEqual(["","next"]);expect(host.textContent).toContain("one");expect(host.textContent).toContain("two");
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="打开笔记：two"]')!.click());expect(select).toHaveBeenCalledWith({id:"two",title:"two"});
});
it("keeps actionable source errors and retries the page",async()=>{
  query.mockRejectedValueOnce(new Error("note_catalog_changed operation=picker-1")).mockResolvedValueOnce({items:[item("retry")]});await mount();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("picker-1");
  await act(async()=>Array.from(host.querySelectorAll("button")).find(button=>button.textContent==="重试")!.click());await act(async()=>vi.advanceTimersByTimeAsync(0));expect(host.textContent).toContain("retry");
});
it("does not publish a late candidate page after the picker closes",async()=>{
  let finish!:(value:NoteTargetCatalogPage)=>void;query.mockReturnValueOnce(new Promise(resolve=>{finish=resolve;}));await mount();
  await act(async()=>root.render(null));await act(async()=>finish({items:[item("late")]}));expect(host.textContent).toBe("");
});
