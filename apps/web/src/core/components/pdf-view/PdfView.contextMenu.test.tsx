// @vitest-environment happy-dom
import { act, createRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { PdfView } from "./PdfView";
import { PdfViewController } from "./PdfViewController";
vi.mock("./usePdfCharacterSelection", () => ({ usePdfCharacterSelection: () => undefined }));
vi.mock("./usePdfZoomGestures", () => ({ usePdfZoomGestures: () => undefined }));
let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => { await cleanup?.(); vi.unstubAllGlobals(); });
it("renders and clears the same controller selection for every input origin",async()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  const host=document.createElement("div"),page=document.createElement("div");document.body.append(host,page);const root=createRoot(host);
  cleanup=async()=>{await act(async()=>root.unmount());host.remove();page.remove();};
  const controller=new PdfViewController(),containerRef=createRef<HTMLDivElement>(),viewerRef=createRef<HTMLDivElement>();
  await act(async()=>root.render(<PdfView controller={controller} containerRef={containerRef} viewerRef={viewerRef} ready active characterSelectionAvailable={false} pageHosts={[{pageIndex:0,element:page}]} documentVersion="fixture"/>));
  const selected={text:"quote",segments:[{pageIndex:0,text:"quote",rects:[{x:.1,y:.2,width:.3,height:.04}],prefix:"",suffix:""}],focusPageIndex:0,focusRect:{x:.1,y:.2,width:.3,height:.04}};
  for(const selection of [selected,{...selected,text:"ordinary"}]){
    await act(async()=>controller.setSelection(selection));expect(page.querySelectorAll(".pending-selection-highlight")).toHaveLength(1);
    const copy=new Event("copy",{bubbles:true,cancelable:true}),setData=vi.fn();
    Object.defineProperty(copy,"clipboardData",{value:{setData}});
    await act(async()=>containerRef.current!.dispatchEvent(copy));
    expect(setData).toHaveBeenCalledExactlyOnceWith("text/plain",selection.text);
    await act(async()=>containerRef.current!.dispatchEvent(new PointerEvent("pointerdown",{bubbles:true,button:2})));
    expect(controller.getSnapshot().selection).toBe(selection);
    await act(async()=>containerRef.current!.dispatchEvent(new PointerEvent("pointerdown",{bubbles:true,button:0})));
    expect(controller.getSnapshot().selection).toBeUndefined();expect(page.querySelector(".pending-selection-highlight")).toBeNull();
  }
});
it("uses page geometry even on portal overlays and leaves non-page context menus alone", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  cleanup = async () => { await act(async () => root.unmount()); host.remove(); };
  const snapshot = { selection: undefined }, selected = vi.fn(), context = vi.fn();
  const adapter = { characterStore: {}, pagePoint: vi.fn(() => ({ pageIndex: 2 })), clientRectForPageRect: vi.fn(() => new DOMRect(100, 200, 400, 600)) };
  const controller = { subscribe: () => () => {}, getSnapshot: () => snapshot, adapter, setSelection: selected } as unknown as PdfViewController;
  const containerRef = createRef<HTMLDivElement>(), viewerRef = createRef<HTMLDivElement>();
  const render = async (active: boolean) => act(async () => root.render(<PdfView controller={controller} containerRef={containerRef} viewerRef={viewerRef} ready active={active} characterSelectionAvailable={false} pageHosts={[]} documentVersion="fixture" onPageContextMenu={context} />));
  await render(true);
  const overlay = document.createElement("button"); viewerRef.current!.append(overlay);
  const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 500 });
  await act(async () => overlay.dispatchEvent(event));
  expect(event.defaultPrevented).toBe(true);
  expect(context).toHaveBeenCalledExactlyOnceWith(2, { x: .5, y: .5, width: .00001, height: .00001 });
  expect(selected).toHaveBeenCalledWith(undefined);
  overlay.className = "saved-position-note";
  await act(async () => overlay.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 500 })));
  expect(context).toHaveBeenCalledTimes(1);
  adapter.pagePoint.mockReturnValueOnce(undefined as unknown as { pageIndex: number });
  const outside = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 30, clientY: 40 });
  await act(async () => containerRef.current!.dispatchEvent(outside)); expect(outside.defaultPrevented).toBe(false);
  await render(false);
  await act(async () => overlay.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 500 })));
  expect(context).toHaveBeenCalledTimes(1);
});
