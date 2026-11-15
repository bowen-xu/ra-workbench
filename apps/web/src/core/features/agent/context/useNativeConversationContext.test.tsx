// @vitest-environment happy-dom
import { act,useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach,expect,it,vi } from "vitest";
import { EMPTY_AGENT_CONTEXT,type AgentContextState } from "@research-assistant/core/agent";
import type { AgentContextGateway } from "../../../services/agentContextGateway";
import { selectAppStorage } from "../../../runtime/appStorage";
import { readConversationContext,writeConversationContext } from "./contextStorage";
import { useNativeConversationContext } from "./useNativeConversationContext";
(globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let dispose:(()=>Promise<void>)|undefined;
const values=new Map<string,string>();
const storage:Storage={get length(){return values.size;},key:index=>[...values.keys()][index]??null,getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},removeItem:key=>{values.delete(key);},clear:()=>values.clear()};
afterEach(async()=>{await dispose?.();dispose=undefined;storage.clear();document.body.innerHTML="";});
it("restores the native internal grant instead of replacing it with an empty client cache",async()=>{
  selectAppStorage(storage);writeConversationContext("internal",EMPTY_AGENT_CONTEXT);
  const state:AgentContextState={...EMPTY_AGENT_CONTEXT,persistent:[{label:"画像",resource:{kind:"structuredObject",resourceId:"portrait",access:"readWrite"}}]};
  const saveConfiguration=vi.fn(),readConfiguration=vi.fn(async()=>({revision:4,state}));
  const gateway={readConfiguration,saveConfiguration} as unknown as AgentContextGateway;
  let rendered:AgentContextState=EMPTY_AGENT_CONTEXT;
  function Harness(){const [value,setValue]=useState(EMPTY_AGENT_CONTEXT);rendered=value;useNativeConversationContext("internal",false,gateway,setValue,vi.fn());return null;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<Harness/>));expect(rendered).toEqual(state);expect(readConversationContext("internal")).toEqual(state);expect(saveConfiguration).not.toHaveBeenCalled();
});
it("preserves an explicitly empty native context instead of resurrecting legacy selections",async()=>{
  selectAppStorage(storage);writeConversationContext("ordinary",{...EMPTY_AGENT_CONTEXT,persistent:[{label:"旧文献",resource:{kind:"paper",resourceId:"old"}}]});
  const saveConfiguration=vi.fn();const gateway={readConfiguration:async()=>({revision:2,state:EMPTY_AGENT_CONTEXT}),saveConfiguration} as unknown as AgentContextGateway;function Harness(){useNativeConversationContext("ordinary",false,gateway,vi.fn(),vi.fn());return null;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<Harness/>));expect(readConversationContext("ordinary")).toEqual(EMPTY_AGENT_CONTEXT);expect(saveConfiguration).not.toHaveBeenCalled();
});

it.each([false,true])("reads archived context without initialization or writes (native row=%s)",async saved=>{
  selectAppStorage(storage);
  const legacy:AgentContextState={...EMPTY_AGENT_CONTEXT,persistent:[{label:"Legacy",resource:{kind:"paper",resourceId:"old"}}]};
  writeConversationContext("archived",legacy);
  const failed=vi.fn(),changed=vi.fn(),saveConfiguration=vi.fn(),readConfiguration=vi.fn(async()=>saved?{revision:2,state:EMPTY_AGENT_CONTEXT}:null);
  const gateway={readConfiguration,saveConfiguration} as unknown as AgentContextGateway;
  let hook!:ReturnType<typeof useNativeConversationContext>;
  function Harness(){hook=useNativeConversationContext("archived",false,gateway,changed,failed,{readOnly:true});return null;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<Harness/>));
  expect(readConfiguration).toHaveBeenCalledExactlyOnceWith("archived",false);
  expect(changed).toHaveBeenLastCalledWith(saved?EMPTY_AGENT_CONTEXT:legacy);
  expect(readConversationContext("archived")).toEqual(saved?EMPTY_AGENT_CONTEXT:legacy);
  expect(hook.ready).toBe(true);expect(failed).not.toHaveBeenCalled();expect(saveConfiguration).not.toHaveBeenCalled();
  await expect(hook.save(EMPTY_AGENT_CONTEXT)).rejects.toThrow("AGENT_CONTEXT_READ_ONLY");
  expect(saveConfiguration).not.toHaveBeenCalled();
});

it("imports a missing archived configuration only after explicit restoration",async()=>{
  selectAppStorage(storage);
  const legacy:AgentContextState={...EMPTY_AGENT_CONTEXT,persistent:[{label:"Legacy",resource:{kind:"paper",resourceId:"old"}}]};
  writeConversationContext("restored",legacy);
  const saveConfiguration=vi.fn(async(_id:string,_revision:number,state:AgentContextState)=>({revision:1,state}));
  const readConfiguration=vi.fn(async()=>null),gateway={readConfiguration,saveConfiguration} as unknown as AgentContextGateway;
  function Harness({readOnly}:{readonly readOnly:boolean}){useNativeConversationContext("restored",false,gateway,vi.fn(),vi.fn(),{readOnly});return null;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<Harness readOnly/>));expect(saveConfiguration).not.toHaveBeenCalled();
  await act(async()=>root.render(<Harness readOnly={false}/>));
  expect(saveConfiguration).toHaveBeenCalledExactlyOnceWith("restored",0,legacy);
  expect(readConversationContext("restored")).toEqual(legacy);
});

it("does not expose the readonly hydration as writable readiness before restoration is rechecked",async()=>{
  selectAppStorage(storage);
  let release!:(value:null)=>void;
  const readConfiguration=vi.fn().mockResolvedValueOnce(null).mockImplementationOnce(()=>new Promise<null>(resolve=>{release=resolve;}));
  const saveConfiguration=vi.fn(async()=>({revision:1,state:EMPTY_AGENT_CONTEXT}));
  const gateway={readConfiguration,saveConfiguration} as unknown as AgentContextGateway,frames:Array<{readOnly:boolean;ready:boolean}>=[];
  function Harness({readOnly}:{readonly readOnly:boolean}){const hook=useNativeConversationContext("restore-ready",false,gateway,vi.fn(),vi.fn(),{readOnly});frames.push({readOnly,ready:hook.ready});return null;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<Harness readOnly/>));expect(frames.at(-1)?.ready).toBe(true);
  await act(async()=>root.render(<Harness readOnly={false}/>));
  expect(frames.find(frame=>!frame.readOnly)?.ready).toBe(false);expect(saveConfiguration).not.toHaveBeenCalled();
  await act(async()=>release(null));expect(frames.at(-1)?.ready).toBe(true);expect(saveConfiguration).toHaveBeenCalledTimes(1);
});

it("revokes legacy import when archiving while its native read is still pending",async()=>{
  selectAppStorage(storage);
  let release!:(value:null)=>void;
  const readConfiguration=vi.fn().mockImplementationOnce(()=>new Promise<null>(resolve=>{release=resolve;})).mockResolvedValue(null);
  const saveConfiguration=vi.fn(),failed=vi.fn(),gateway={readConfiguration,saveConfiguration} as unknown as AgentContextGateway;
  function Harness({readOnly}:{readonly readOnly:boolean}){useNativeConversationContext("switching",false,gateway,vi.fn(),failed,{readOnly});return null;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<Harness readOnly={false}/>));
  await act(async()=>root.render(<Harness readOnly/>));
  await act(async()=>release(null));
  expect(saveConfiguration).not.toHaveBeenCalled();expect(failed).not.toHaveBeenCalled();
  expect(readConfiguration).toHaveBeenLastCalledWith("switching",false);
});

it("keeps an already-started save but revokes queued writes when the same conversation is archived",async()=>{
  selectAppStorage(storage);
  let release!:(value:{revision:number;state:AgentContextState})=>void;
  const saving=new Promise<{revision:number;state:AgentContextState}>(resolve=>{release=resolve;});
  const saveConfiguration=vi.fn(()=>saving),gateway={readConfiguration:async()=>({revision:1,state:EMPTY_AGENT_CONTEXT}),saveConfiguration} as unknown as AgentContextGateway;
  let hook!:ReturnType<typeof useNativeConversationContext>;
  function Harness({readOnly}:{readonly readOnly:boolean}){hook=useNativeConversationContext("queued",false,gateway,vi.fn(),vi.fn(),{readOnly});return null;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<Harness readOnly={false}/>));
  let first!:Promise<void>,rejected!:Promise<void>;
  await act(async()=>{first=hook.save(EMPTY_AGENT_CONTEXT);rejected=expect(hook.save(EMPTY_AGENT_CONTEXT)).rejects.toThrow("AGENT_CONTEXT_READ_ONLY");});
  await act(async()=>root.render(<Harness readOnly/>));
  await act(async()=>{release({revision:2,state:EMPTY_AGENT_CONTEXT});await first;await rejected;});
  expect(saveConfiguration).toHaveBeenCalledTimes(1);
});

it("finishes a queued save under its old owner after switching without changing the new UI or revision",async()=>{
  selectAppStorage(storage);
  const stateA:AgentContextState={...EMPTY_AGENT_CONTEXT,persistent:[{label:"A",resource:{kind:"paper",resourceId:"paper-a"}}]};
  const stateB:AgentContextState={...EMPTY_AGENT_CONTEXT,persistent:[{label:"B",resource:{kind:"paper",resourceId:"paper-b"}}]};
  let resolveA!: (value:{revision:number;state:AgentContextState})=>void;
  const savingA=new Promise<{revision:number;state:AgentContextState}>(resolve=>{resolveA=resolve;});
  const saveConfiguration=vi.fn(async(id:string,revision:number,state:AgentContextState)=>id === "a"?savingA:{revision:revision+1,state});
  const gateway={readConfiguration:async(id:string)=>({revision:id === "a"?5:9,state:id === "a"?stateA:stateB}),saveConfiguration} as unknown as AgentContextGateway;
  let hook!:ReturnType<typeof useNativeConversationContext>;let rendered=EMPTY_AGENT_CONTEXT;const changed=vi.fn();
  function Harness({id}:{readonly id:string}){const [value,setValue]=useState(EMPTY_AGENT_CONTEXT);rendered=value;hook=useNativeConversationContext(id,false,gateway,state=>{changed(state);setValue(state);},vi.fn());return null;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<Harness id="a"/>));
  let pending!:Promise<void>;await act(async()=>{pending=hook.save(stateA);});
  await act(async()=>root.render(<Harness id="b"/>));changed.mockClear();
  await act(async()=>{resolveA({revision:6,state:stateA});await pending;});
  expect(rendered).toEqual(stateB);expect(changed).not.toHaveBeenCalled();
  await act(async()=>{await hook.save(stateB);});
  expect(saveConfiguration.mock.calls.map(call=>call.slice(0,2))).toEqual([["a",5],["b",9]]);
});

it("keeps a failed import editable and becomes ready after the user saves a corrected selection",async()=>{
  selectAppStorage(storage);const legacy:AgentContextState={...EMPTY_AGENT_CONTEXT,persistent:[{label:"Removed",resource:{kind:"paper",resourceId:"deleted"}}]};writeConversationContext("conversation",legacy);
  const failed=vi.fn(),saveConfiguration=vi.fn().mockRejectedValueOnce(new Error("sourceUnavailable")).mockResolvedValueOnce({revision:1,state:EMPTY_AGENT_CONTEXT});
  const gateway={readConfiguration:async()=>null,saveConfiguration} as unknown as AgentContextGateway;
  let hook!:ReturnType<typeof useNativeConversationContext>;
  function Harness(){hook=useNativeConversationContext("conversation",false,gateway,vi.fn(),failed);return null;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<Harness/>));expect(hook.ready).toBe(false);expect(hook.loading).toBe(false);expect(readConversationContext("conversation")).toEqual(legacy);
  await act(async()=>hook.save(EMPTY_AGENT_CONTEXT));expect(hook.ready).toBe(true);expect(readConversationContext("conversation")).toEqual(EMPTY_AGENT_CONTEXT);expect(failed).toHaveBeenCalledOnce();
});

it("reloads native authority after a resource-created grant without racing an in-flight context save", async () => {
  selectAppStorage(storage);
  const added: AgentContextState = {...EMPTY_AGENT_CONTEXT, persistent: [{
    label: "Created template", resource: {kind: "structuredObject", resourceId: "new-template", access: "readWrite"},
  }]};
  let readCount = 0;
  let changedEvent!: () => void;
  let resolveSave!: (value: {revision: number; state: AgentContextState}) => void;
  const saving = new Promise<{revision: number; state: AgentContextState}>(resolve => { resolveSave = resolve; });
  const gateway = {
    readConfiguration: vi.fn(async () => ({revision: ++readCount === 1 ? 1 : 3,
      state: readCount === 1 ? EMPTY_AGENT_CONTEXT : added})),
    saveConfiguration: vi.fn(() => saving),
    subscribeConfiguration: async (_id: string, changed: () => void) => {
      changedEvent = changed;
      return () => undefined;
    },
  } as unknown as AgentContextGateway;
  let hook!: ReturnType<typeof useNativeConversationContext>;
  let displayed = EMPTY_AGENT_CONTEXT;
  function Harness() {
    hook = useNativeConversationContext("conversation", false, gateway, state => { displayed = state; }, vi.fn());
    return null;
  }
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  dispose = () => act(async () => root.unmount());
  await act(async () => root.render(<Harness/>));
  let pending!: Promise<void>;
  await act(async () => { pending = hook.save(EMPTY_AGENT_CONTEXT); changedEvent(); });
  expect(readCount).toBe(1);
  await act(async () => { resolveSave({revision: 2, state: EMPTY_AGENT_CONTEXT}); await pending; });
  expect(displayed).toEqual(added);
  expect(readConversationContext("conversation")).toEqual(added);
});
