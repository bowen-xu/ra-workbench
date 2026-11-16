/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useReaderSourceSelection } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/selection/useReaderSourceSelection";
import type { ReaderSourceNavigation } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/ReaderSourceNavigation";
import type { PdfCharacterSelection } from "../../../../../../../apps/web/src/core/components/pdf-view/pdfPageCharacterModel";
import { PdfViewController } from "../../../../../../../apps/web/src/core/components/pdf-view/PdfViewController";

const hash="a".repeat(64),rect={x:.1,y:.2,width:.3,height:.04};
const selection:PdfCharacterSelection={pageIndex:0,start:0,end:5,text:"quote",prefix:"",suffix:"",rects:[rect],
  lastClientRect:{left:10,top:20,right:40,bottom:24,width:30,height:4},focusPageIndex:0,focusRect:rect,
  segments:[{pageIndex:0,start:0,end:5,text:"quote",prefix:"",suffix:"",rects:[rect],lastClientRect:{left:10,top:20,right:40,bottom:24,width:30,height:4}}]};
const source:ReaderSourceNavigation={paperId:"paper",paperTitle:"Paper",attachmentId:"pdf",contentSha256:hash,
  pageIndex:0,rects:[],quoteExact:"quote",quotePrefix:"",quoteSuffix:"",state:"exact",
  navigationLineRange:{startLine:1,startColumn:1,endLine:1,endColumn:6}};
const controller={viewController:new PdfViewController(),resolveLineSelection:vi.fn(async()=>selection),resolveQuoteSelection:vi.fn(async()=>selection),navigate:vi.fn()};
type Options=Parameters<typeof useReaderSourceSelection>[0];
let root:ReturnType<typeof createRoot>,host:HTMLDivElement,result:ReturnType<typeof useReaderSourceSelection>;
function Harness(props:Options){result=useReaderSourceSelection(props);return result.selection?<span>selected</span>:null;}
const options:Options={source,contentSha256:hash,controller,ready:true,pageCount:1,pageHosts:[],announce:vi.fn(),beginSelection:vi.fn()};
async function render(value:Options=options){await act(async()=>root.render(<StrictMode><Harness {...value}/></StrictMode>));}
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);vi.clearAllMocks();controller.viewController=new PdfViewController();controller.resolveLineSelection.mockResolvedValue(selection);controller.resolveQuoteSelection.mockResolvedValue(selection);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
it("clears a located range and never restores it on page-host updates, but a new navigation can select it again",async()=>{
  await render();expect(result.selection).toBe(selection);expect(result.locatedSource).toBe(source);
  expect(controller.viewController.getSnapshot().selection).toBe(selection);
  await act(async()=>controller.viewController.clearSelection());expect(result.selection).toBeUndefined();
  controller.resolveLineSelection.mockClear();controller.navigate.mockClear();
  await render({...options,pageHosts:[{pageIndex:0,element:host}]});
  expect(result.selection).toBeUndefined();expect(controller.resolveLineSelection).not.toHaveBeenCalled();expect(controller.navigate).not.toHaveBeenCalled();
  await render({...options,source:{...source}});expect(result.selection).toBe(selection);
});
it("ignores a late resolution after the user dismisses a pending navigation",async()=>{
  let finish!:(value:PdfCharacterSelection)=>void;
  controller.resolveLineSelection.mockImplementation(()=>new Promise(resolve=>{finish=resolve;}));
  await render();await act(async()=>controller.viewController.clearSelection());await act(async()=>finish(selection));
  expect(result.selection).toBeUndefined();expect(controller.navigate).not.toHaveBeenCalled();
});
it("uses the same store for a new ordinary selection and prevents a late navigation from replacing it",async()=>{
  let finish!:(value:PdfCharacterSelection)=>void;
  controller.resolveLineSelection.mockImplementation(()=>new Promise(resolve=>{finish=resolve;}));
  await render();
  const manual={...selection,text:"ordinary selection"};
  await act(async()=>controller.viewController.setSelection(manual));
  expect(result.selection).toBe(manual);
  await render({...options,pageHosts:[{pageIndex:0,element:host}]});
  await act(async()=>finish(selection));
  expect(result.selection).toBe(manual);expect(controller.viewController.getSnapshot().selection).toBe(manual);
});
it("keeps quote-only navigation dismissed across repaint as well",async()=>{
  const {navigationLineRange:_range,...quote}=source;await render({...options,source:quote});expect(result.selection).toBe(selection);
  await act(async()=>controller.viewController.clearSelection());controller.resolveQuoteSelection.mockClear();
  await render({...options,source:quote,pageHosts:[{pageIndex:0,element:host}]});
  expect(result.selection).toBeUndefined();expect(controller.resolveQuoteSelection).not.toHaveBeenCalled();
});
it("does not reuse an old result after navigation changes or select a mismatched attachment",async()=>{
  let finish!:(value:PdfCharacterSelection)=>void;
  controller.resolveLineSelection.mockImplementation(()=>new Promise(resolve=>{finish=resolve;}));
  await render();const old=finish;
  controller.resolveLineSelection.mockResolvedValue(selection);await render({...options,source:{...source,quoteExact:"new"}});
  await act(async()=>old({...selection,text:"stale"}));expect(result.selection?.text).toBe("quote");
  controller.resolveLineSelection.mockClear();await render({...options,contentSha256:"b".repeat(64)});
  expect(result.selection).toBeUndefined();expect(controller.resolveLineSelection).not.toHaveBeenCalled();
});
