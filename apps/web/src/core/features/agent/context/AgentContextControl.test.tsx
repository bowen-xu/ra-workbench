// @vitest-environment happy-dom
import {act,StrictMode,useState} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it,vi} from "vitest";
import {EMPTY_AGENT_CONTEXT,selectAgentContext,type AgentContextSelection,type AgentContextState} from "@research-assistant/core/agent";
import {AgentContextControl} from "./AgentContextControl";
import {useContextRecommendations} from "./useContextRecommendations";
import {selectAppStorage} from "../../../runtime/appStorage";
import {readConversationContext,writeConversationContext} from "./contextStorage";
import {recommendationDraftKey} from "./contextRecommendationDraft";
import {contextDirectoryGateway} from "./contextDirectoryGateway";
import {notify} from "../../../services/notifications";
const group:AgentContextSelection={label:"Fixture paper",resource:{kind:"paperGroup",resourceId:"paper",parts:[
  {resource:{kind:"paperMetadata",resourceId:"paper"},label:"元数据",selected:true,access:"readOnly"},
  {resource:{kind:"paper",resourceId:"paper",attachmentId:"pdf"},label:"paper.pdf",selected:true,access:"readOnly"},
  {resource:{kind:"paperNotes",resourceId:"paper",excludedNoteIds:[]},label:"阅读笔记",selected:true,access:"readOnly"},
  {resource:{kind:"note",resourceId:"note"},label:"阅读笔记 · 440",selected:false,access:"readOnly"},
  {resource:{kind:"paperPortrait",resourceId:"paper"},label:"文献画像",selected:true,access:"readOnly"},
]}};
const observations=vi.hoisted(()=>({open:true}));
vi.mock("./AgentContextProvider",()=>({useAgentContextCandidates:()=>observations.open?[group]:[],useAgentContextSuggestions:()=>observations.open?[group]:[]}));
vi.mock("./contextDirectoryGateway",()=>({contextDirectoryGateway:{read:async()=>null,set:async()=>null}}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
const values=new Map<string,string>();
const storage:Storage={get length(){return values.size;},key:index=>[...values.keys()][index]??null,getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},removeItem:key=>{values.delete(key);},clear:()=>values.clear()};
let cleanup:()=>Promise<void>;
afterEach(async()=>{await cleanup?.();storage.clear();observations.open=true;vi.useRealTimers();});
it.each(["label", "permission"])("retains a later recommendation %s change when an earlier request finishes",async change=>{
  selectAppStorage(storage);
  storage.setItem(recommendationDraftKey("fixture"),JSON.stringify([{item:group,selected:true}]));
  const host=document.createElement("div"),root=createRoot(host);document.body.append(host);
  let recommendations!:ReturnType<typeof useContextRecommendations>;
  function Harness(){recommendations=useContextRecommendations("fixture",EMPTY_AGENT_CONTEXT);return null;}
  cleanup=async()=>{await act(async()=>root.unmount());host.remove();};
  await act(async()=>root.render(<Harness/>));
  recommendations.capture("request");
  if(group.resource.kind!=="paperGroup") throw new Error("invalid fixture");
  const later=change==="label"?{...group,label:"Updated paper"}:{...group,resource:{...group.resource,
    parts:group.resource.parts.map((part,index)=>index===0?{...part,access:"readWrite"}:part)}};
  storage.setItem(recommendationDraftKey("fixture"),JSON.stringify([{item:later,selected:true}]));
  await act(async()=>recommendations.completeCommit("request"));
  expect(JSON.parse(storage.getItem(recommendationDraftKey("fixture"))!)).toEqual([{item:later,selected:true}]);
});
it("hides the configured recommendation after admission while preserving it in context management and after remount",async()=>{
  vi.useFakeTimers();selectAppStorage(storage);
  writeConversationContext("fixture",selectAgentContext(EMPTY_AGENT_CONTEXT,group,"persistent"));
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const gateway={catalog:async()=>[group],search:async()=>[group]};
  function Harness({initialPending}:{initialPending:boolean}){
    const [state,setState]=useState(()=>readConversationContext("fixture")),[pending,setPending]=useState(initialPending);
    const recommendations=useContextRecommendations("fixture",state,pending);
    return <><button onClick={()=>{recommendations.capture("request");setState(recommendations.commit("request"));setPending(false);}}>接纳消息</button>
      <AgentContextControl state={state} recommendations={recommendations} pending={pending} onChange={setState} gateway={gateway as never} disabled={false} conversationId="fixture"/></>;
  }
  cleanup=async()=>{await act(async()=>root.unmount());host.remove();};
  await act(async()=>root.render(<Harness key="draft" initialPending/>));
  expect(host.querySelector('[aria-label="推荐持续上下文 Fixture paper"]')).not.toBeNull();
  await act(async()=>host.querySelector<HTMLButtonElement>("button")!.click());
  expect(host.querySelector('.agent-context-recommendation')).toBeNull();
  expect(readConversationContext("fixture").persistent).toHaveLength(1);
  await act(async()=>root.render(<Harness key="restored" initialPending={false}/>));
  expect(host.querySelector('.agent-context-recommendation')).toBeNull();
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="上下文管理"]')!.click());
  await act(async()=>vi.advanceTimersByTimeAsync(200));
  expect(document.querySelector<HTMLInputElement>('[aria-label="使用 Fixture paper · paper.pdf"]')!.checked).toBe(true);
  expect(document.querySelector<HTMLInputElement>('[aria-label="使用 Fixture paper · 阅读笔记"]')!.checked).toBe(true);
});
it("moves configured recommendations into context management and preserves reading-note exclusions",async()=>{
  vi.useFakeTimers();selectAppStorage(storage);
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const gateway={catalog:async()=>[group],search:async()=>[group]};
  function Harness(){const [state,setState]=useState(EMPTY_AGENT_CONTEXT);const recommendations=useContextRecommendations("fixture",state);return <AgentContextControl state={state} recommendations={recommendations} onChange={setState} gateway={gateway as never} disabled={false} conversationId="fixture"/>;}
  cleanup=async()=>{await act(async()=>root.unmount());host.remove();};
  await act(async()=>root.render(<Harness/>));
  async function click(label:string){const button=document.querySelector<HTMLElement>(`[aria-label="${label}"]`);expect(button).not.toBeNull();await act(async()=>button!.click());}
  await click("添加推荐上下文 Fixture paper");await click("上下文管理");await act(async()=>vi.advanceTimersByTimeAsync(200));
  const checked=(label:string)=>document.querySelector<HTMLInputElement>(`[aria-label="${label}"]`)!.checked;
  expect(checked("使用 Fixture paper · paper.pdf")).toBe(true);expect(checked("使用 Fixture paper · 元数据")).toBe(true);expect(checked("使用 Fixture paper · 阅读笔记")).toBe(true);
  await click("使用 Fixture paper · 元数据");expect(checked("使用 Fixture paper · 元数据")).toBe(false);
  expect(host.querySelector('.agent-context-recommendation')).toBeNull();
  await click("使用 Fixture paper");expect(checked("使用 Fixture paper · paper.pdf")).toBe(true);
  expect(checked("使用 Fixture paper · 元数据")).toBe(true);
  await act(async()=>{const summary=[...document.querySelectorAll("summary")].find(node=>node.textContent?.startsWith("排除项"));summary!.click();});
  await click("排除 阅读笔记 · 440 note");expect(checked("排除 阅读笔记 · 440 note")).toBe(true);expect(checked("使用 Fixture paper · 阅读笔记")).toBe(true);
  await click("使用 Fixture paper · paper.pdf");expect(checked("排除 阅读笔记 · 440 note")).toBe(true);
});
it("hides the header recommendation after closing all resources while keeping selected resources in context management",async()=>{
  vi.useFakeTimers();selectAppStorage(storage);
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const gateway={catalog:async()=>[group],search:async()=>[group]};
  function Harness(){const [state,setState]=useState(()=>selectAgentContext(EMPTY_AGENT_CONTEXT,group,"persistent"));const recommendations=useContextRecommendations("fixture",state,true);return <AgentContextControl state={state} recommendations={recommendations} onChange={setState} gateway={gateway as never} disabled={false} conversationId="fixture" pending/>;}
  cleanup=async()=>{await act(async()=>root.unmount());host.remove();};
  const render=()=>act(async()=>root.render(<StrictMode><Harness/></StrictMode>));
  await render();
  expect(host.querySelector('[aria-label="移除推荐上下文 Fixture paper"]')).not.toBeNull();
  observations.open=false;await render();
  expect(host.querySelector('.agent-context-recommendation')).toBeNull();
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="上下文管理"]')!.click());
  await act(async()=>vi.advanceTimersByTimeAsync(200));
  expect(document.querySelector<HTMLInputElement>('[aria-label="使用 Fixture paper · paper.pdf"]')!.checked).toBe(true);
  expect(document.querySelector<HTMLInputElement>('[aria-label="使用 Fixture paper · 阅读笔记"]')!.checked).toBe(true);
  await act(async()=>document.querySelector<HTMLButtonElement>('[aria-label="关闭上下文管理"]')!.click());
  observations.open=true;await render();
  expect(host.querySelector('[aria-label="移除推荐上下文 Fixture paper"]')).not.toBeNull();
});

it.each([true,false])("saves a delayed directory change and navigates only while its owner is current: %s",async(current)=>{
  vi.useFakeTimers(); selectAppStorage(storage);
  const project:AgentContextSelection={label:"Project",resource:{kind:"manuscript",resourceId:"project",groupAccess:"readWrite",newMemberAccess:"readWrite",parts:[]}};
  let release!:(value:{manuscriptId:string;title:string;path:string})=>void;
  const pending=new Promise<{manuscriptId:string;title:string;path:string}>(resolve=>{release=resolve;});
  const saveDirectory=vi.spyOn(contextDirectoryGateway,"set").mockReturnValue(pending);
  const saveContext=vi.fn(async(_next:AgentContextState)=>{}), navigate=vi.fn();
  let valid=true; const capture=vi.fn(()=>()=>valid);
  const host=document.createElement("div"),root=createRoot(host);document.body.append(host);
  const gateway={catalog:async()=>[project],search:async()=>[project]};
  function Harness(){const [state,setState]=useState(EMPTY_AGENT_CONTEXT);const recommendations=useContextRecommendations("fixture",state);
    return <AgentContextControl state={state} recommendations={recommendations} onChange={async next=>{await saveContext(next);setState(next);}} gateway={gateway as never} disabled={false} conversationId="fixture" captureDirectoryNavigation={capture} onDirectoryChanged={navigate}/>;}
  cleanup=async()=>{await act(async()=>root.unmount());host.remove();saveDirectory.mockRestore();};
  await act(async()=>root.render(<Harness/>));
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="上下文管理"]')!.click());
  await act(async()=>vi.advanceTimersByTimeAsync(200));
  const input=document.querySelector<HTMLInputElement>('[aria-label="使用 Project"]');expect(input).not.toBeNull();
  await act(async()=>input!.click());
  expect(capture).toHaveBeenCalledOnce();expect(saveDirectory).toHaveBeenCalledOnce();
  valid=current;
  await act(async()=>release({manuscriptId:"project",title:"Project",path:"main.tex"}));
  expect(saveContext).toHaveBeenCalledOnce();
  if(current) expect(navigate).toHaveBeenCalledExactlyOnceWith({manuscriptId:"project",title:"Project",path:"main.tex"});
  else expect(navigate).not.toHaveBeenCalled();
});

it.each([true,false])("reports catalog failures through the original notification owner: native=%s",async native=>{
  vi.useFakeTimers();selectAppStorage(storage);
  const failure=native?{code:"plugin_data_sql_failed",message:"文献上下文目录来源不可用。",
    packageKey:"a".repeat(64),phase:"read",safeCause:"sqlite_failure",
    resourceId:"literature.context-catalog.papers",resourceVersion:1,sqliteExtendedCode:5}:"unavailable";
  const report=vi.spyOn(notify,"error").mockReturnValue("catalog-error");
  const host=document.createElement("div"),root=createRoot(host);document.body.append(host);
  const gateway={catalog:async()=>{throw failure;},search:async()=>[]};
  function Harness(){const recommendations=useContextRecommendations("fixture",EMPTY_AGENT_CONTEXT);
    return <AgentContextControl state={EMPTY_AGENT_CONTEXT} recommendations={recommendations} onChange={()=>undefined}
      gateway={gateway as never} disabled={false} conversationId="fixture"/>;}
  cleanup=async()=>{await act(async()=>root.unmount());host.remove();report.mockRestore();};
  await act(async()=>root.render(<Harness/>));
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="上下文管理"]')!.click());
  await act(async()=>vi.advanceTimersByTimeAsync(200));
  expect(report).toHaveBeenCalledOnce();
  const message=report.mock.calls[0]![0];
  if(native){expect(message).toContain("code=plugin_data_sql_failed");expect(message).toContain(`packageKey=${"a".repeat(64)}`);}
  else expect(message).toBe("无法读取材料，请重试。");
});
