/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PdfSelectionActions } from "../../../../../../../../plugins/ra-literature-library/src/features/knowledge/PdfSelectionActions";
import { useReaderSelectionSessions, type ReaderSelectionSessions } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/selection/useReaderSelectionSessions";
import { submitSelectionAnnotation } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/selection/submitSelectionAnnotation";
import { PdfViewController } from "../../../../../../../apps/web/src/core/components/pdf-view/PdfViewController";
import { SpeechInputAdapterProvider } from "../../../../../../../apps/web/src/core/features/speech/SpeechInputAdapter";
import { writingDemoSpeechAdapter } from "../../../../../../../apps/web/src/app/previews/agent/WritingDemoSpeechAdapter";

type Options=Parameters<typeof useReaderSelectionSessions>[0];
const rect={x:.1,y:.2,width:.3,height:.04};
const initial={id:"selected",active:true,dirty:false,pending:{text:"原文",prefix:"",suffix:"",pageIndex:0,pageLabel:"1",rects:[rect],
  segments:[{pageIndex:0,pageLabel:"1",rects:[rect],text:"原文",prefix:"",suffix:""}],popupPageIndex:0,popupRect:rect}};
let host:HTMLDivElement,root:ReturnType<typeof createRoot>,sessions:ReaderSelectionSessions;
let controller:Options["controller"];
const submit=vi.fn(async()=>undefined),fallback=vi.fn();
function Harness(){
  sessions=useReaderSelectionSessions({active:true,viewerReady:true,characterSelectionAvailable:true,pageLabels:["1"],
    item:{id:"paper",title:"Paper"} as Options["item"],attachment:{id:"pdf",contentSha256:"a".repeat(64)} as Options["attachment"],controller,
    annotationController:{} as Options["annotationController"],knowledgeGateway:{} as Options["knowledgeGateway"],referenceGateway:{} as Options["referenceGateway"],
    viewerContainerRef:{current:host},setAnnouncement:vi.fn(),initialSessions:[initial]});
  const session=sessions.activeSession;
  return session?<PdfSelectionActions key={session.id} active pending={session.pending} paperId="paper" knowledgeGateway={{} as Options["knowledgeGateway"]}
    onHighlight={async()=>undefined} onWriteNote={async()=>undefined} onCaptureConcept={async()=>undefined} onCaptureReference={async()=>undefined}
    onEnsureAiAnchor={async()=>"anchor"} onAiRunStarted={vi.fn()}
    onDraftChange={(draft,dirty)=>sessions.updateDraftState(session.id,draft,dirty)}
    onAnnotate={(comment,intent)=>submitSelectionAnnotation({paperId:"paper",attachmentId:"pdf",title:"paper.pdf",pending:session.pending,controller,
      comment,...(intent?{intent}:{}),submit,discard:()=>sessions.discard(session.id)})}/>:null;
}
beforeEach(async()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);vi.clearAllMocks();submit.mockResolvedValue(undefined);
  controller={viewController:new PdfViewController(),renderPageRegionPng:vi.fn(),setStatus:vi.fn()} as unknown as Options["controller"];
  host=document.createElement("div");document.body.append(host);root=createRoot(host);
  window.addEventListener("keydown",fallback);
  await act(async()=>root.render(<SpeechInputAdapterProvider adapter={writingDemoSpeechAdapter}><Harness/></SpeechInputAdapterProvider>));
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="问AI"]')!.click());
});
afterEach(async()=>{window.removeEventListener("keydown",fallback);await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
async function type(value:string){await act(async()=>{const input=host.querySelector("textarea")!;Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,value);input.dispatchEvent(new Event("input",{bubbles:true}));});}
it.each(["textarea","button"])("Esc from %s applies the comment and ends the session without creating a reader draft",async target=>{
  await type("我的评论");expect(sessions.hasDirtyDrafts()).toBe(false);
  await act(async()=>{const control=host.querySelector(target)!;control.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true}));});
  expect(submit).toHaveBeenCalledExactlyOnceWith("",expect.any(Array),{mode:"draft",reveal:false,evidence:[expect.objectContaining({text:"原文",comment:"我的评论"})]});
  expect(fallback).not.toHaveBeenCalled();expect(host.querySelector("textarea")).toBeNull();
  expect(sessions.sessions).toEqual([]);expect(sessions.drafts).toEqual([]);expect(controller.viewController.getSnapshot().selection).toBeUndefined();
});
it("keeps a failed apply visible for retry without folding it into a reader draft",async()=>{
  await type("保留评论");submit.mockRejectedValueOnce(new Error("暂时不能应用"));
  const escape=()=>host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true}));
  await act(async()=>{escape();});expect(host.querySelector("textarea")?.value).toBe("保留评论");expect(sessions.drafts).toEqual([]);
  await act(async()=>{escape();});expect(submit).toHaveBeenCalledTimes(2);expect(sessions.sessions).toEqual([]);
});
