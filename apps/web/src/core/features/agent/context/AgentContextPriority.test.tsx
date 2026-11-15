// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {EMPTY_AGENT_CONTEXT,type AgentContextSelection} from "@research-assistant/core/agent";
import {AgentContextProvider,useAgentContextCandidate,useAgentContextCandidates} from "./AgentContextProvider";
import {useContextRecommendations} from "./useContextRecommendations";
import {AgentContextRecommendations} from "./AgentContextRecommendations";
const recommend=vi.hoisted(()=>vi.fn());
vi.mock("../../../services/agentContextGateway",()=>({agentContextGateway:{recommend}}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
const selection=(id:string):AgentContextSelection=>({label:id,resource:{kind:"note",resourceId:id}});

it("resolves the open project through the gateway and refreshes it after project resource changes",async()=>{
  let finish!:(value:AgentContextSelection)=>void,candidates:readonly AgentContextSelection[]=[];
  recommend.mockImplementation(()=>new Promise(resolve=>{finish=resolve;}));
  const candidate:AgentContextSelection={label:"Project",resource:{kind:"manuscript",resourceId:"project",parts:[]}};
  const resolved:AgentContextSelection={...candidate,resource:{...candidate.resource,kind:"manuscript",parts:[],groupAccess:"readWrite",newMemberAccess:"readWrite"}};
  function Project(){useAgentContextCandidate(candidate);candidates=useAgentContextCandidates();return null;}
  const root=createRoot(document.createElement("div"));
  try{
    await act(async()=>root.render(<AgentContextProvider><Project/></AgentContextProvider>));
    expect(recommend).toHaveBeenCalledWith(candidate,expect.any(AbortSignal));
    expect(candidates).toEqual([]);
    await act(async()=>finish(resolved));expect(candidates).toEqual([resolved]);
    await act(async()=>window.dispatchEvent(new Event("research-assistant:writing-notes-changed")));
    expect(recommend).toHaveBeenCalledTimes(2);
    expect(candidates).toEqual([]);
    await act(async()=>finish(resolved));expect(candidates).toEqual([resolved]);
  }finally{await act(async()=>root.unmount());recommend.mockReset();}
});

it.each([true,false])("keeps the active tab primary and the floating note folded (floating resolves first: %s)",async floatingFirst=>{
  const pending=new Map<string,(value:AgentContextSelection)=>void>();
  recommend.mockImplementation((item:AgentContextSelection)=>new Promise(resolve=>pending.set(item.resource.resourceId,resolve)));
  let candidates:readonly AgentContextSelection[]=[], items:readonly AgentContextSelection[]=[];
  function Document({id,surface}:{id:string;surface:"tab"|"floating"}){useAgentContextCandidate(selection(id),true,surface);return null;}
  function Recommendations(){
    candidates=useAgentContextCandidates();
    const result=useContextRecommendations(null,EMPTY_AGENT_CONTEXT,true);items=result.items;
    return items[0]?<AgentContextRecommendations primary={items[0]} selected={[]} suggestions={items} disabled={false} onChange={async()=>undefined}/>:null;
  }
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  try{
    await act(async()=>root.render(<AgentContextProvider><Document id="tab" surface="tab"/><Document id="floating" surface="floating"/><Recommendations/></AgentContextProvider>));
    const first=floatingFirst?"floating":"tab",second=floatingFirst?"tab":"floating";
    await act(async()=>pending.get(first)!(selection(first)));
    expect(candidates).toEqual(floatingFirst?[]:[selection("tab")]);
    if(floatingFirst)expect(items).toEqual([]);
    await act(async()=>pending.get(second)!(selection(second)));
    expect(candidates).toEqual([selection("tab")]);expect(items).toEqual([selection("tab"),selection("floating")]);
    expect(host.querySelector('[aria-label="推荐持续上下文 tab"]')).not.toBeNull();
    expect(host.querySelector('[aria-label="添加推荐上下文 floating"]')).toBeNull();
    await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="推荐上下文 tab"]')!.click());
    expect(document.querySelector('[aria-label="添加持续上下文 floating"]')).not.toBeNull();
    expect(recommend).toHaveBeenCalledTimes(2);
  }finally{await act(async()=>root.unmount());host.remove();recommend.mockReset();}
});

it("prefers the tab's label for a shared identity and falls back only when no active tab remains",async()=>{
  recommend.mockImplementation(async(item:AgentContextSelection)=>item);
  let candidates:readonly AgentContextSelection[]=[];
  function Document({surface,label}:{surface:"tab"|"floating";label:string}){useAgentContextCandidate({...selection("same"),label},true,surface);return null;}
  function Observe(){candidates=useAgentContextCandidates();return null;}
  const root=createRoot(document.createElement("div"));
  try{
    await act(async()=>root.render(<AgentContextProvider><Document key="tab" surface="tab" label="Tab title"/><Document key="floating" surface="floating" label="Floating title"/><Observe/></AgentContextProvider>));
    expect(candidates.map(item=>item.label)).toEqual(["Tab title"]);
    await act(async()=>root.render(<AgentContextProvider><Document key="floating" surface="floating" label="Floating title"/><Observe/></AgentContextProvider>));
    expect(candidates.map(item=>item.label)).toEqual(["Floating title"]);
  }finally{await act(async()=>root.unmount());recommend.mockReset();}
});

it("merges floating literature notes into the paper already recommended by its tab",async()=>{
  recommend.mockImplementation(async(item:AgentContextSelection)=>({label:"Paper",resource:{kind:"paperGroup",resourceId:item.resource.resourceId,parts:[],groupAccess:"readWrite",newMemberAccess:"readWrite"}}));
  let candidates:readonly AgentContextSelection[]=[],suggestions:readonly AgentContextSelection[]=[];
  function Document({surface,attachmentId}:{surface:"tab"|"floating";attachmentId?:string}){
    useAgentContextCandidate({label:"Paper",resource:{kind:"paper",resourceId:"paper",...(attachmentId?{attachmentId}:{})}},true,surface);return null;
  }
  function Observe(){candidates=useAgentContextCandidates();suggestions=useContextRecommendations(null,EMPTY_AGENT_CONTEXT,true).items;return null;}
  const root=createRoot(document.createElement("div"));
  try{
    await act(async()=>root.render(<AgentContextProvider><Document surface="tab" attachmentId="pdf"/><Document surface="floating"/><Document surface="floating"/><Observe/></AgentContextProvider>));
    expect(candidates.map(item=>item.resource)).toEqual([{kind:"paperGroup",resourceId:"paper",parts:[],groupAccess:"readWrite",newMemberAccess:"readWrite"}]);
    expect(suggestions).toEqual(candidates);
  }finally{await act(async()=>root.unmount());recommend.mockReset();}
});
