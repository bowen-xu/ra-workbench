// @vitest-environment happy-dom
import {act,memo} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import type {AgentContextSelection} from "@research-assistant/core/agent";
import {AgentContextProvider,useAgentContextCandidate,useAgentContextCandidates,useAgentContextSuggestions} from "./AgentContextProvider";
const recommend=vi.hoisted(()=>vi.fn());
vi.mock("../../../services/agentContextGateway",()=>({agentContextGateway:{recommend}}));

it("does not broadcast candidate changes to passive resource observers",async()=>{
  (globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
  const root=createRoot(document.createElement("div")), passive=vi.fn();
  const Passive=memo(function Passive(){passive();useAgentContextCandidate({label:"Hidden",resource:{kind:"paper",resourceId:"hidden"}},false);return null;});
  function Current({id}:{id:string}){useAgentContextCandidate({label:id,resource:{kind:"paper",resourceId:id}});return null;}
  recommend.mockImplementation(async (value:AgentContextSelection)=>value);
  const render=(id:string)=>act(async()=>root.render(<AgentContextProvider><Passive/><Current id={id}/></AgentContextProvider>));
  try {
    await render("a");passive.mockClear();await render("b");await render("a");
    expect(passive).not.toHaveBeenCalled();
  } finally {await act(async()=>root.unmount());recommend.mockReset();}
});

it.each(["note","manuscriptFile"] as const)("recommends the writable managed group for an opened %s",async kind=>{
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
  const host=document.createElement("div"),root=createRoot(host);document.body.append(host);
  const leaf:AgentContextSelection={label:"Current",resource:kind==="note"?{kind,resourceId:"id"}:{kind,resourceId:"id",path:"main.tex"}};
  const group:AgentContextSelection={label:"Group",resource:{kind:kind==="note"?"noteGroup":"manuscript",resourceId:"id",groupAccess:"readWrite",newMemberAccess:"readWrite",parts:[]}};
  recommend.mockResolvedValue(group);
  let observed:readonly AgentContextSelection[]=[];
  function Current(){useAgentContextCandidate(leaf);return null;}
  function Observe(){observed=useAgentContextCandidates();return null;}
  try {
    await act(async()=>root.render(<AgentContextProvider><Current/><Observe/></AgentContextProvider>));
    expect(observed).toEqual([group]);
    expect(recommend).toHaveBeenLastCalledWith(leaf,expect.any(AbortSignal));
  } finally {await act(async()=>root.unmount());host.remove();recommend.mockReset();}
});

it("switches activation without expiry queries and refreshes on focus or changed identities",async()=>{
  (globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
  const clock=vi.spyOn(Date,"now").mockReturnValue(100);
  const host=document.createElement("div"),root=createRoot(host);document.body.append(host);
  const leaf=(id:string):AgentContextSelection=>({label:id,resource:{kind:"paper",resourceId:id,attachmentId:"pdf"}});
  const group=(id:string):AgentContextSelection=>({label:id,resource:{kind:"paperGroup",resourceId:id,groupAccess:"readWrite",parts:[]}});
  recommend.mockImplementation(async(value:AgentContextSelection)=>group(value.resource.resourceId));
  let activeRows:readonly AgentContextSelection[]=[],suggestions:readonly AgentContextSelection[]=[];
  function Current({active,id}:{active:boolean;id:string}){useAgentContextCandidate(leaf(id),active);return null;}
  function Observe(){activeRows=useAgentContextCandidates();suggestions=useAgentContextSuggestions();return null;}
  const render=(active:boolean,id="paper")=>act(async()=>root.render(<AgentContextProvider><Current active={active} id={id}/><Observe/></AgentContextProvider>));
  try {
    await render(true);expect(recommend).toHaveBeenCalledTimes(1);
    for(let i=0;i<4;i++){
      await render(false);expect(activeRows).toEqual([]);expect(suggestions).toEqual([group("paper")]);
      await render(true);expect(activeRows).toEqual([group("paper")]);
    }
    expect(recommend).toHaveBeenCalledTimes(1);
    clock.mockReturnValue(5_101);await render(false);await render(true);
    expect(recommend).toHaveBeenCalledTimes(1);
    await act(async()=>window.dispatchEvent(new Event("focus"))); expect(recommend).toHaveBeenCalledTimes(2);
    await render(true,"other");expect(activeRows).toEqual([group("other")]);
    expect(recommend).toHaveBeenCalledTimes(3);
    await act(async()=>root.render(<AgentContextProvider><Observe/></AgentContextProvider>));
    expect(activeRows).toEqual([]);expect(suggestions).toEqual([]);
  } finally {await act(async()=>root.unmount());host.remove();clock.mockRestore();recommend.mockReset();}
});

it("coalesces concurrent observers and invalidates only the changed note recommendation", async () => {
  (globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
  const root = createRoot(document.createElement("div"));
  recommend.mockImplementation(async (value:AgentContextSelection)=>value);
  function Current({id, active = true}: {id: string; active?: boolean}) {useAgentContextCandidate({label:id, resource:{kind:"note",resourceId:id}}, active); return null;}
  try {
    await act(async()=>root.render(<AgentContextProvider><Current id="a"/><Current id="a"/><Current id="b"/></AgentContextProvider>));
    expect(recommend).toHaveBeenCalledTimes(2); recommend.mockClear();
    await act(async()=>window.dispatchEvent(new CustomEvent("research-assistant:note-resources-changed", {detail:{pageId:"a"}})));
    expect(recommend).toHaveBeenCalledExactlyOnceWith({label:"a",resource:{kind:"note",resourceId:"a"}},expect.any(AbortSignal));
    recommend.mockClear();
    await act(async()=>window.dispatchEvent(new CustomEvent("research-assistant:note-resources-changed", {detail:{pageId:"a",revision:2}})));
    expect(recommend).not.toHaveBeenCalled();
    await act(async()=>root.render(<AgentContextProvider><Current id="a" active={false}/><Current id="a" active={false}/><Current id="b"/></AgentContextProvider>));
    await act(async()=>root.render(<AgentContextProvider><Current id="a"/><Current id="a"/><Current id="b"/></AgentContextProvider>));
    expect(recommend).toHaveBeenCalledExactlyOnceWith({label:"a",resource:{kind:"note",resourceId:"a"}},expect.any(AbortSignal));
  } finally {await act(async()=>root.unmount()); recommend.mockReset();}
});

it("keeps a slow recommendation on backgrounding and never substitutes its raw leaf", async () => {
  (globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
  const leaf:AgentContextSelection={label:"Synthetic note",resource:{kind:"note",resourceId:"note"}};
  const group:AgentContextSelection={label:"Synthetic group",resource:{kind:"noteGroup",resourceId:"note",groupAccess:"readWrite",newMemberAccess:"readWrite",parts:[]}};
  let finish!:(value:AgentContextSelection)=>void;
  recommend.mockImplementation(()=>new Promise(resolve=>{finish=resolve;}));
  let activeRows:readonly AgentContextSelection[]=[], opened:readonly AgentContextSelection[]=[];
  function Current({active}:{active:boolean}){useAgentContextCandidate(leaf,active);return null;}
  function Observe(){activeRows=useAgentContextCandidates();opened=useAgentContextSuggestions();return null;}
  const root=createRoot(document.createElement("div"));
  const render=(active:boolean)=>act(async()=>root.render(<AgentContextProvider><Current active={active}/><Observe/></AgentContextProvider>));
  try {
    await render(true);await render(false);
    expect(activeRows).toEqual([]);expect(opened).toEqual([]);
    expect(recommend).toHaveBeenCalledTimes(1);
    await act(async()=>finish(group));
    expect(activeRows).toEqual([]);expect(opened).toEqual([group]);
    await render(true);expect(activeRows).toEqual([group]);
    expect(recommend).toHaveBeenCalledTimes(1);
  } finally {await act(async()=>root.unmount());recommend.mockReset();}
});

it("cancels obsolete identities and never republishes a closed resource",async()=>{
  (globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
  const pending=new Map<string,{finish:(value:AgentContextSelection)=>void;signal:AbortSignal}>();
  recommend.mockImplementation((value:AgentContextSelection,signal:AbortSignal)=>new Promise(resolve=>pending.set(value.resource.resourceId,{finish:resolve,signal})));
  let opened:readonly AgentContextSelection[]=[];
  function Current({id}:{id:string}){useAgentContextCandidate({label:id,resource:{kind:"note",resourceId:id}});return null;}
  function Observe(){opened=useAgentContextSuggestions();return null;}
  const root=createRoot(document.createElement("div"));
  const render=(id:string|null)=>act(async()=>root.render(<AgentContextProvider>{id&&<Current id={id}/>}<Observe/></AgentContextProvider>));
  try {
    await render("old");await render("new");expect(pending.get("old")!.signal.aborted).toBe(true);
    await act(async()=>pending.get("old")!.finish({label:"old",resource:{kind:"note",resourceId:"old"}}));
    expect(opened).toEqual([]);
    await render(null);expect(pending.get("new")!.signal.aborted).toBe(true);
    await act(async()=>pending.get("new")!.finish({label:"new",resource:{kind:"note",resourceId:"new"}}));
    expect(opened).toEqual([]);
  } finally {await act(async()=>root.unmount());recommend.mockReset();}
});
