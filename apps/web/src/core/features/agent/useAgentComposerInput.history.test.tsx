/** @vitest-environment happy-dom */
import { act, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AgentInputDraft } from "@research-assistant/core/agent";
import type { AgentComposerInputHandle } from "./AgentComposerTextInput";
import { useAgentComposerInput } from "./useAgentComposerInput";
vi.mock("./useAgentReferencePicker",()=>({useAgentReferencePicker:()=>({update:vi.fn(),keyDown:()=>false,close:vi.fn()})}));
vi.mock("./useAgentCommandPicker",()=>({useAgentCommandPicker:()=>({edited:vi.fn(),update:vi.fn(),keyDown:()=>false,close:vi.fn()})}));
vi.mock("./evidenceImageGateway",()=>({retainHistoryEvidence:vi.fn(),releaseHistoryEvidence:vi.fn()}));

it("undoes badge clear, comment edits and text locally, blocks propagation and resets on external submission",async()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  const host=document.createElement("div"),root=createRoot(host);document.body.append(host);
  const initial:AgentInputDraft={text:"问题",bindings:[],evidence:[{type:"annotation",source:"source://notes/note:#block{b}",title:"引用",text:"原文",comment:"旧评论"}]};
  let api!:ReturnType<typeof useAgentComposerInput>, external!: (draft:AgentInputDraft)=>void;
  function Harness(){
    const [draft,setDraft]=useState(initial),input=useRef<AgentComposerInputHandle>(null);external=setDraft;
    api=useAgentComposerInput({text:draft.text,setText:text=>setDraft(value=>({...value,text})),input,access:{draft:()=>draft,set:setDraft}});
    return <div onKeyDown={event=>api.historyKeyDown(event)}><button>clear</button><output>{JSON.stringify(api.draft)}</output></div>;
  }
  const mainUndo=vi.fn();window.addEventListener("keydown",mainUndo);
  const key=async(key:string,shift=false)=>{await act(async()=>host.querySelector("button")!.dispatchEvent(new KeyboardEvent("keydown",{key,ctrlKey:true,shiftKey:shift,bubbles:true,cancelable:true})));};
  try{
    await act(async()=>root.render(<Harness/>));
    await act(async()=>api.changeDraft({...api.draft,evidence:[]}));
    await key("z");expect(api.draft.evidence).toEqual(initial.evidence);expect(mainUndo).not.toHaveBeenCalled();
    await key("z",true);expect(api.draft.evidence).toEqual([]);
    await key("z");await act(async()=>api.changeDraft({...api.draft,evidence:[{...initial.evidence![0]!,comment:"新评论"}]}));
    await key("z");expect(api.draft.evidence).toEqual(initial.evidence);
    await act(async()=>api.change("问题abc"));await key("z");expect(api.draft.text).toBe("问题");
    await act(async()=>external({text:"",bindings:[]}));await key("z");expect(api.draft).toEqual({text:"",bindings:[]});
  }finally{await act(async()=>root.unmount());host.remove();window.removeEventListener("keydown",mainUndo);vi.unstubAllGlobals();}
});
