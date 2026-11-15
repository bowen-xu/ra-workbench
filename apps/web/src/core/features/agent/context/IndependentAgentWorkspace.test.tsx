// @vitest-environment happy-dom
import { act, useEffect, type ReactNode, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { IndependentAgentWorkspace as CoreIndependentAgentWorkspace } from "./IndependentAgentWorkspace";
import { WritingDiffDemoHistory } from "../../../../app/previews/agent/WritingDiffDemoHistory";
import { selectAppStorage } from "../../../runtime/appStorage";
import type { AgentContextGateway } from "../../../services/agentContextGateway";
import { notify } from "../../../services/notifications";


import { AgentConversationAdaptersProvider } from "./AgentConversationAdapters";
function WritingSession({id}:{id:string}) {return <section data-writing-session={id}/>;}
function IndependentAgentWorkspace(props: ComponentProps<typeof CoreIndependentAgentWorkspace>) {
  return <AgentConversationAdaptersProvider WritingSession={WritingSession}><CoreIndependentAgentWorkspace {...props}/></AgentConversationAdaptersProvider>;
}

const ports = vi.hoisted(() => ({ metadata: vi.fn(), directory: vi.fn(), deliveries: vi.fn() }));
vi.mock("../../../services/agentGateway", () => ({
  agentMetadataGateway: { read: ports.metadata }, agentCatalogGateway: {},
  agentArchiveGateway: {}, agentSearchGateway: {}, agentDeletionGateway: {}, agentConversationExportGateway: {},
  agentSessionGateway: {}, agentTurnGateway: {}, agentTaskObservationGateway: {},
}));
vi.mock("./contextDirectoryGateway", () => ({ contextDirectoryGateway: { read: ports.directory } }));
vi.mock("./AgentContextProvider", () => ({ useAgentContextCandidates: () => [] }));
vi.mock("../BoundConversationList", () => ({ BoundConversationList: ({onSelect,onCreate}: {onSelect: (id: string, archived: boolean) => void; onCreate: () => void}) => <div data-testid="conversation-list"><button onClick={() => onSelect("22222222-2222-4222-8222-222222222222", false)}>选择稿件对话</button><button onClick={() => onSelect("33333333-3333-4333-8333-333333333333", false)}>选择独立对话</button><button onClick={onCreate}>新对话</button></div> }));
vi.mock("./IndependentAgentSession", () => ({ IndependentAgentSession: ({id, pending, messageRequest, ensureCreated}: {id: string; pending: boolean; messageRequest?: import("../AgentCanvasMessage").AgentCanvasMessage | null; ensureCreated:()=>Promise<void>}) => {
  useEffect(() => { if (messageRequest) ports.deliveries(id, messageRequest.text); }, [id, messageRequest]);
  return <section data-session={id} data-pending={pending}><input aria-label="消息"/><button onClick={async()=>{await ensureCreated();await ensureCreated();}}>保存并绑定</button></section>;
} }));
vi.mock("../presentation/WritingAgentPanel", () => ({ WritingAgentPanel: ({historyHeader, inputDisabled}: {historyHeader: ReactNode; inputDisabled: boolean}) => <section>{historyHeader}<input aria-label="消息" disabled={inputDisabled}/></section> }));

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
const oldId = "11111111-1111-4111-8111-111111111111";
const selectedKey = "research-assistant:independent-selected";
const pendingKey = "research-assistant:conversation-pending";
const values = new Map<string, string>();
const storage: Storage = {get length() {return values.size;}, key: index => [...values.keys()][index] ?? null,
  getItem: key => values.get(key) ?? null, setItem: (key, value) => {values.set(key, value);},
  removeItem: key => {values.delete(key);}, clear: () => values.clear()};
vi.stubGlobal("localStorage", storage);
let cleanup: (() => Promise<void>) | undefined;
beforeEach(() => {
  vi.spyOn(notify,"error").mockReturnValue("workspace-error");
  localStorage.clear(); selectAppStorage(localStorage);
  ports.deliveries.mockReset(); ports.metadata.mockReset(); ports.directory.mockReset().mockResolvedValue(null);
});
afterEach(async () => { await cleanup?.(); cleanup = undefined; localStorage.clear(); vi.restoreAllMocks(); });
async function mount(open = true, workspaceActive = false, showList = false) {
  const history = new WritingDiffDemoHistory();
  const create = vi.spyOn(history.gateway, "create");
  const gateway: AgentContextGateway = {
    sessions: history.gateway, list: async () => [], search: async () => [],
    prepare: async () => { throw new Error("unused"); }, restore: async () => { throw new Error("unused"); },
    start: async () => { throw new Error("unused"); }, interrupt: async () => undefined,
  };
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const reveal=vi.fn(), select=vi.fn();
  const render = async (draftRequest = 0, messageRequest?: import("../AgentCanvasMessage").AgentCanvasMessage, active = workspaceActive) => { await act(async () => root.render(<IndependentAgentWorkspace messageRequest={messageRequest ?? null} draftRequest={draftRequest} workspaceActive={active} open={open} onOpenChange={reveal} onSelectContext={select} headerControls={null} drawer={content => showList ? content : null} gateway={gateway}/>)); };
  cleanup = async () => { await act(async () => root.unmount()); host.remove(); history.dispose(); };
  await render();
  return {host, create, render, reveal, select};
}
it("does not create a hidden independent draft while the writing host is active", async () => {
  const {render, create} = await mount(true, true);
  expect(localStorage.getItem(pendingKey)).toBeNull(); expect(localStorage.getItem(selectedKey)).toBeNull();
  await render(0, undefined, false);
  expect(localStorage.getItem(pendingKey)).toBeTruthy(); expect(create).not.toHaveBeenCalled();
});
it("unlocks the list when a writing selection supersedes a pending independent restore", async () => {
  localStorage.setItem(selectedKey, oldId);
  let finish!: (value: {id: string; title: string; archived: boolean}) => void;
  ports.metadata.mockImplementation(({id}: {id: string}) => id === oldId
    ? new Promise(resolve => {finish = resolve;}) : Promise.resolve({id, title: "Writing", archived: false}));
  const history = new WritingDiffDemoHistory();
  const gateway: AgentContextGateway = {sessions: history.gateway, list: async () => [], search: async () => [],
    prepare: async () => {throw new Error("unused");}, restore: async () => {throw new Error("unused");},
    start: async () => {throw new Error("unused");}, interrupt: async () => undefined};
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  cleanup = async () => {await act(async () => root.unmount()); host.remove(); history.dispose();};
  await act(async () => root.render(<IndependentAgentWorkspace workspaceActive externalId="22222222-2222-4222-8222-222222222222" open onOpenChange={() => undefined}
    headerControls={null} drawer={content => content} gateway={gateway}/>));
  expect(host.querySelector('[data-testid="conversation-list"]')?.parentElement?.hasAttribute("inert")).toBe(false);
  await act(async () => finish({id: oldId, title: "Old", archived: false}));
  expect(host.querySelector('[data-testid="conversation-list"]')?.parentElement?.hasAttribute("inert")).toBe(false);
});
it("does not restore or create an independent conversation over pending manuscript navigation", async () => {
  localStorage.setItem(selectedKey, oldId);
  let finish!: (value: {id: string; title: string; archived: boolean}) => void;
  ports.metadata.mockReturnValue(new Promise(resolve => {finish = resolve;}));
  const history = new WritingDiffDemoHistory();
  const gateway: AgentContextGateway = {sessions: history.gateway, list: async () => [], search: async () => [],
    prepare: async () => {throw new Error("unused");}, restore: async () => {throw new Error("unused");},
    start: async () => {throw new Error("unused");}, interrupt: async () => undefined};
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host), select = vi.fn();
  cleanup = async () => {await act(async () => root.unmount()); host.remove(); history.dispose();};
  const render = async (navigationPending: boolean) => {await act(async () => root.render(<IndependentAgentWorkspace navigationPending={navigationPending} open onOpenChange={() => undefined} onSelectContext={select} headerControls={null} drawer={content => content} gateway={gateway}/>));};
  await render(false); await render(true);
  await act(async () => finish({id: oldId, title: "Old", archived: false}));
  expect(select).not.toHaveBeenCalled();
  expect(localStorage.getItem(pendingKey)).toBeNull();
  expect(host.querySelector('[data-testid="conversation-list"]')?.closest("[inert]")).toBeNull();
  await render(false);
  expect(localStorage.getItem(pendingKey)).toBeTruthy();
});
it("keeps the latest list selection when an earlier manuscript navigation ends", async () => {
  localStorage.setItem(selectedKey, oldId);
  const second = "33333333-3333-4333-8333-333333333333";
  let finish!: (value: {id: string; title: string; archived: boolean}) => void;
  ports.metadata.mockImplementation(({id}: {id: string}) => id === second
    ? new Promise(resolve => {finish = resolve;})
    : Promise.resolve({id, title: "Old", archived: false}));
  const history = new WritingDiffDemoHistory();
  const gateway: AgentContextGateway = {sessions: history.gateway, list: async () => [], search: async () => [],
    prepare: async () => {throw new Error("unused");}, restore: async () => {throw new Error("unused");},
    start: async () => {throw new Error("unused");}, interrupt: async () => undefined};
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host), select = vi.fn(), started = vi.fn();
  cleanup = async () => {await act(async () => root.unmount()); host.remove(); history.dispose();};
  const render = async (navigationPending: boolean) => {await act(async () => root.render(<IndependentAgentWorkspace navigationPending={navigationPending} open onOpenChange={() => undefined} onSelectionStarted={started} onSelectContext={select} headerControls={null} drawer={content => content} gateway={gateway}/>));};
  await render(false); await render(true); select.mockClear();
  const oldReads = ports.metadata.mock.calls.filter(([arg]) => arg.id === oldId).length;
  await act(async () => host.querySelectorAll<HTMLButtonElement>('[data-testid="conversation-list"] button')[1]!.click());
  expect(started).toHaveBeenCalledOnce();
  await render(false);
  expect(ports.metadata.mock.calls.filter(([arg]) => arg.id === oldId)).toHaveLength(oldReads);
  await act(async () => finish({id: second, title: "Second", archived: false}));
  expect(select).toHaveBeenCalledExactlyOnceWith(second, null, false);
});
it("allows another selection while metadata is pending and ignores the older response", async () => {
  const first = "22222222-2222-4222-8222-222222222222";
  const second = "33333333-3333-4333-8333-333333333333";
  let resolveFirst!: (value: {id:string; title:string; archived:boolean}) => void;
  ports.metadata.mockImplementation(({id}: {id:string}) => id === first
    ? new Promise(resolve => {resolveFirst = resolve;})
    : Promise.resolve({id, title:"Second", archived:false}));
  const {host, select} = await mount(true, true, true);
  const buttons = host.querySelectorAll<HTMLButtonElement>('[data-testid="conversation-list"] button');
  await act(async () => buttons[0]!.click());
  expect(buttons[0]!.closest("[inert]")).toBeNull();
  await act(async () => buttons[1]!.click());
  expect(select).toHaveBeenCalledExactlyOnceWith(second, null, false);
  await act(async () => resolveFirst({id:first, title:"First", archived:false}));
  expect(select).toHaveBeenCalledTimes(1);
  expect(ports.directory).not.toHaveBeenCalledWith(first);
  expect(localStorage.getItem(selectedKey)).toBe(second);
});
it("allows another selection while directory is pending and ignores the older result", async () => {
  const first = "22222222-2222-4222-8222-222222222222";
  const second = "33333333-3333-4333-8333-333333333333";
  let resolveFirst!: (value: {manuscriptId:string; title:string; path:string}) => void;
  ports.metadata.mockImplementation(async ({id}: {id:string}) => ({id, title:"Saved", archived:false}));
  ports.directory.mockImplementation((id:string) => id === first
    ? new Promise(resolve => {resolveFirst = resolve;})
    : Promise.resolve(null));
  const {host, select} = await mount(true, true, true);
  const buttons = host.querySelectorAll<HTMLButtonElement>('[data-testid="conversation-list"] button');
  await act(async () => buttons[0]!.click());
  expect(buttons[0]!.closest("[inert]")).toBeNull();
  await act(async () => buttons[1]!.click());
  expect(select).toHaveBeenCalledExactlyOnceWith(second, null, false);
  await act(async () => resolveFirst({manuscriptId:"paper", title:"Paper", path:"main.tex"}));
  expect(select).toHaveBeenCalledTimes(1);
  expect(localStorage.getItem(selectedKey)).toBe(second);
});
it("keeps an explicit selection in flight when the active manuscript gateway changes", async () => {
  const first = "22222222-2222-4222-8222-222222222222";
  let resolveDirectory!: (value: {manuscriptId:string; title:string; path:string}) => void;
  ports.metadata.mockImplementation(async ({id}: {id:string}) => ({id, title:"Saved", archived:false}));
  ports.directory.mockImplementation((id:string) => id === first
    ? new Promise(resolve => {resolveDirectory = resolve;}) : Promise.resolve(null));
  const history = new WritingDiffDemoHistory();
  const gateway: AgentContextGateway = {sessions:history.gateway, list:async()=>[], search:async()=>[],
    prepare:async()=>{throw new Error("unused");}, restore:async()=>{throw new Error("unused");},
    start:async()=>{throw new Error("unused");}, interrupt:async()=>undefined};
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const selected = vi.fn(), reveal = vi.fn();
  cleanup = async () => {await act(async () => root.unmount()); host.remove(); history.dispose();};
  const render = async (current:AgentContextGateway) => act(async () => root.render(<IndependentAgentWorkspace workspaceActive open onOpenChange={reveal}
    onSelectContext={selected} headerControls={null} drawer={content => content} gateway={current}/>));
  await render(gateway);
  await act(async () => host.querySelector<HTMLButtonElement>('[data-testid="conversation-list"] button')!.click());
  await render({...gateway});
  const directory = {manuscriptId:"paper",title:"Paper",path:"main.tex"};
  await act(async () => resolveDirectory(directory));
  expect(selected).toHaveBeenCalledExactlyOnceWith(first,directory,false);
  expect(localStorage.getItem(selectedKey)).toBe(first);
});
it("uses the current navigation callbacks after an asynchronous selection", async () => {
  const first = "22222222-2222-4222-8222-222222222222";
  let resolveDirectory!: (value: {manuscriptId:string; title:string; path:string}) => void;
  ports.metadata.mockImplementation(async ({id}: {id:string}) => ({id,title:"Saved",archived:false}));
  ports.directory.mockReturnValue(new Promise(resolve => {resolveDirectory = resolve;}));
  const history = new WritingDiffDemoHistory();
  const gateway: AgentContextGateway = {sessions:history.gateway, list:async()=>[], search:async()=>[],
    prepare:async()=>{throw new Error("unused");}, restore:async()=>{throw new Error("unused");},
    start:async()=>{throw new Error("unused");}, interrupt:async()=>undefined};
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  cleanup = async () => {await act(async () => root.unmount()); host.remove(); history.dispose();};
  const oldSelect = vi.fn(), newSelect = vi.fn(), oldOpen = vi.fn(), newOpen = vi.fn();
  const render = async (select:typeof oldSelect, open:typeof oldOpen) => act(async () => root.render(<IndependentAgentWorkspace
    workspaceActive open onOpenChange={open} onSelectContext={select} headerControls={null} drawer={content => content} gateway={gateway}/>));
  await render(oldSelect,oldOpen);
  await act(async () => host.querySelector<HTMLButtonElement>('[data-testid="conversation-list"] button')!.click());
  await render(newSelect,newOpen);
  const directory = {manuscriptId:"paper",title:"Paper",path:"main.tex"};
  await act(async () => resolveDirectory(directory));
  expect(oldSelect).not.toHaveBeenCalled(); expect(oldOpen).not.toHaveBeenCalled();
  expect(newSelect).toHaveBeenCalledExactlyOnceWith(first,directory,false);
  expect(newOpen).toHaveBeenCalledExactlyOnceWith(true);
});
it("restores a saved manuscript conversation in chat without navigating its project", async () => {
  localStorage.setItem(selectedKey,oldId);
  ports.metadata.mockResolvedValue({id:oldId,title:"Saved",archived:false});
  const directory = {manuscriptId:"paper",title:"Paper",path:"main.tex"};
  ports.directory.mockResolvedValue(directory);
  const {select,host} = await mount(true,false);
  expect(select).toHaveBeenCalledWith(oldId,directory,false);
  expect(host.querySelector('[data-writing-session]')?.getAttribute("data-writing-session")).toBe(oldId);
});
it("does not open the writing host when restoring a closed chat", async () => {
  localStorage.setItem(selectedKey,oldId);
  ports.metadata.mockResolvedValue({id:oldId,title:"Saved",archived:false});
  ports.directory.mockResolvedValue({manuscriptId:"paper",title:"Paper",path:"main.tex"});
  const {select,reveal} = await mount(false,false);
  expect(select).not.toHaveBeenCalled(); expect(reveal).not.toHaveBeenCalled();
});
it("ignores a late selection failure after a newer selection succeeds", async () => {
  const first = "22222222-2222-4222-8222-222222222222";
  const second = "33333333-3333-4333-8333-333333333333";
  let rejectFirst!: (reason: Error) => void;
  ports.metadata.mockImplementation(({id}: {id:string}) => id === first
    ? new Promise((_, reject) => {rejectFirst = reject;})
    : Promise.resolve({id, title:"Second", archived:false}));
  const {host, select} = await mount(true, true, true);
  const buttons = host.querySelectorAll<HTMLButtonElement>('[data-testid="conversation-list"] button');
  await act(async () => buttons[0]!.click());
  await act(async () => buttons[1]!.click());
  await act(async () => rejectFirst(new Error("unavailable")));
  expect(select).toHaveBeenCalledExactlyOnceWith(second, null, false);
  expect(notify.error).not.toHaveBeenCalled();
});
it("does not retry a stale failure after a newer selection succeeds", async () => {
  const first = "22222222-2222-4222-8222-222222222222";
  const second = "33333333-3333-4333-8333-333333333333";
  ports.metadata.mockImplementation(({id}: {id:string}) => id === first
    ? Promise.reject(new Error("unavailable"))
    : Promise.resolve({id, title:"Second", archived:false}));
  const {host, select} = await mount(true, true, true);
  const buttons = host.querySelectorAll<HTMLButtonElement>('[data-testid="conversation-list"] button');
  await act(async () => buttons[0]!.click());
  const action=vi.mocked(notify.error).mock.lastCall?.[1]?.action;
  await act(async () => buttons[1]!.click());
  if (action && typeof action === "object" && "onClick" in action && typeof action.onClick === "function")
    await act(async () => {(action.onClick as () => void)();});
  else throw new Error("Missing retry action");
  expect(select).toHaveBeenCalledExactlyOnceWith(second, null, false);
  expect(localStorage.getItem(selectedKey)).toBe(second);
});
it("preserves the selected conversation when the latest directory lookup fails", async () => {
  const first = "22222222-2222-4222-8222-222222222222";
  const second = "33333333-3333-4333-8333-333333333333";
  let resolveFirst!: (value: {manuscriptId:string; title:string; path:string}) => void;
  ports.metadata.mockImplementation(async ({id}: {id:string}) => ({id, title:"Saved", archived:false}));
  ports.directory.mockImplementation((id:string) => id === first
    ? new Promise(resolve => {resolveFirst = resolve;})
    : Promise.reject(new Error("directory unavailable")));
  const {host, select} = await mount(true, true, true);
  const buttons = host.querySelectorAll<HTMLButtonElement>('[data-testid="conversation-list"] button');
  await act(async () => buttons[0]!.click());
  await act(async () => buttons[1]!.click());
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("无法核对对话及上下文"), expect.any(Object));
  expect(buttons[0]!.closest("[inert]")).toBeNull();
  await act(async () => resolveFirst({manuscriptId:"paper", title:"Paper", path:"main.tex"}));
  expect(select).not.toHaveBeenCalled();
  expect(localStorage.getItem(selectedKey)).toBeNull();
  ports.directory.mockResolvedValue(null);
  const action=vi.mocked(notify.error).mock.lastCall?.[1]?.action;
  if (action && typeof action === "object" && "onClick" in action && typeof action.onClick === "function")
    await act(async () => {(action.onClick as () => void)();});
  else throw new Error("Missing retry action");
  expect(select).toHaveBeenCalledExactlyOnceWith(second, null, false);
  expect(localStorage.getItem(selectedKey)).toBe(second);
});
it("ignores a pending selection after creating a new conversation", async () => {
  const first = "22222222-2222-4222-8222-222222222222";
  let resolveFirst!: (value: {id:string; title:string; archived:boolean}) => void;
  ports.metadata.mockReturnValue(new Promise(resolve => {resolveFirst = resolve;}));
  const {host, select} = await mount(true, true, true);
  const buttons = host.querySelectorAll<HTMLButtonElement>('[data-testid="conversation-list"] button');
  await act(async () => buttons[0]!.click());
  await act(async () => buttons[2]!.click());
  const created = localStorage.getItem(selectedKey);
  expect(created).toBeTruthy();
  expect(select).toHaveBeenCalledExactlyOnceWith(created, null, false);
  await act(async () => resolveFirst({id:first, title:"First", archived:false}));
  expect(localStorage.getItem(selectedKey)).toBe(created);
  expect(select).toHaveBeenCalledTimes(1);
});
it("selects the writing adapter immediately without waiting for a project page", async () => {
  localStorage.setItem(selectedKey, oldId);
  ports.metadata.mockImplementation(async ({id}: {id: string}) => ({id, title: "Saved", archived: false}));
  const writingId = "22222222-2222-4222-8222-222222222222";
  const directory = {manuscriptId: "manuscript", title: "稿件", path: "main.tex"};
  ports.directory.mockImplementation(async (id: string) => id === writingId ? directory : null);
  const history = new WritingDiffDemoHistory();
  const gateway: AgentContextGateway = {sessions: history.gateway, list: async () => [], search: async () => [],
    prepare: async () => {throw new Error("unused");}, restore: async () => {throw new Error("unused");},
    start: async () => {throw new Error("unused");}, interrupt: async () => undefined};
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const select = vi.fn();
  cleanup = async () => {await act(async () => root.unmount()); host.remove(); history.dispose();};
  await act(async () => root.render(<IndependentAgentWorkspace open onOpenChange={() => undefined} onSelectContext={select}
    headerControls={null} drawer={content => content} gateway={gateway}/>));
  expect(host.querySelector('[data-session]')?.getAttribute("data-session")).toBe(oldId);
  await act(async () => host.querySelector<HTMLButtonElement>('[data-testid="conversation-list"] button')!.click());
  expect(select).toHaveBeenCalledWith(writingId, directory, false);
  expect(host.querySelector(`[data-writing-session="${writingId}"]`)).not.toBeNull();
  await act(async () => host.querySelectorAll<HTMLButtonElement>('[data-testid="conversation-list"] button')[1]!.click());
  expect(host.querySelector('[data-session]')?.getAttribute("data-session")).toBe("33333333-3333-4333-8333-333333333333");
  expect(select).toHaveBeenLastCalledWith("33333333-3333-4333-8333-333333333333", null, false);
});
it("retains the selected domain identity and rejects a send when no writing execution adapter is installed", async () => {
  localStorage.setItem(selectedKey, oldId);
  ports.metadata.mockResolvedValue({id:oldId, title:"保留的稿件会话", archived:false});
  ports.directory.mockResolvedValue({manuscriptId:"manuscript", title:"稿件", path:"main.tex"});
  const history = new WritingDiffDemoHistory();
  const create = vi.spyOn(history.gateway,"create");
  const gateway: AgentContextGateway = {sessions:history.gateway,list:async()=>[],search:async()=>[],
    prepare:async()=>{throw new Error("unused");},restore:async()=>{throw new Error("unused");},
    start:async()=>{throw new Error("unused");},interrupt:async()=>undefined};
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  cleanup=async()=>{await act(async()=>root.unmount());host.remove();history.dispose();};
  const rejected=vi.fn();
  await act(async()=>root.render(<CoreIndependentAgentWorkspace open headerControls={null} drawer={()=>null}
    onOpenChange={()=>undefined} gateway={gateway}
    messageRequest={{text:"保留输入",resolve:vi.fn(),reject:rejected}}/>));
  expect(localStorage.getItem(selectedKey)).toBe(oldId);
  expect(host.querySelector<HTMLInputElement>('input[aria-label="消息"]')?.disabled).toBe(true);
  expect(host.querySelector("[data-writing-session]")).toBeNull();
  expect(create).not.toHaveBeenCalled();
  expect(rejected).toHaveBeenCalledWith(expect.objectContaining({code:"agent_execution_adapter_unavailable"}));
});
it("creates a first local draft for silent capture without revealing chat or creating a durable conversation",async()=>{
  const {create,render,reveal,select}=await mount(false);
  expect(localStorage.getItem(pendingKey)).toBeNull();
  const request={text:"",options:{mode:"draft" as const,reveal:false as const,evidence:[{type:"annotation" as const,title:"原文",source:"source://library/paper/pdf",text:"选段",comment:""}]},resolve:vi.fn(),reject:vi.fn()};
  await render(0,request);
  const next=localStorage.getItem(selectedKey);expect(next).toBeTruthy();expect(localStorage.getItem(pendingKey)).toBe(next);
  expect(ports.deliveries).toHaveBeenCalledExactlyOnceWith(next, "");
  expect(create).not.toHaveBeenCalled();expect(reveal).not.toHaveBeenCalled();expect(select).not.toHaveBeenCalled();
});
it("does not recreate an admitted draft when a directory change reuses an earlier callback",async()=>{
  ports.metadata.mockImplementation(async({id}:{id:string})=>({id,title:"Saved",archived:false}));
  const {host,create}=await mount();
  await act(async()=>host.querySelector<HTMLButtonElement>("button")!.click());
  expect(create).toHaveBeenCalledTimes(1);
  expect(localStorage.getItem(pendingKey)).toBeNull();
});
it("automatically replaces an obsolete selection with an unsent draft and preserves old recovery data", async () => {
  localStorage.setItem(selectedKey, oldId);
  const receiptKey = `research-assistant:independent-pending:${oldId}`;
  localStorage.setItem(receiptKey, "old receipt");
  ports.metadata.mockRejectedValue("notFound");
  const {host, create} = await mount();
  const next = localStorage.getItem(selectedKey);
  expect(next).not.toBe(oldId); expect(next).toBeTruthy();
  expect(localStorage.getItem(pendingKey)).toBe(next);
  expect(host.querySelector('[data-pending="true"]')).not.toBeNull();
  expect(host.querySelector<HTMLInputElement>("input")?.disabled).toBe(false);
  expect(host.textContent).not.toContain("新建对话");
  expect(localStorage.getItem(receiptKey)).toBe("old receipt");
  expect(create).not.toHaveBeenCalled(); expect(ports.directory).not.toHaveBeenCalled();
});
it("keeps an unavailable selection and restores the same conversation through retry", async () => {
  localStorage.setItem(selectedKey, oldId);
  ports.metadata.mockRejectedValueOnce("unavailable").mockResolvedValue({id: oldId, title: "Saved", archived: false});
  const {host, create} = await mount();
  expect(localStorage.getItem(selectedKey)).toBe(oldId);
  expect(localStorage.getItem(pendingKey)).toBeNull();
  expect(host.textContent).not.toContain("新建对话");
  expect(host.querySelector<HTMLInputElement>("input")?.disabled).toBe(true);
  const action=vi.mocked(notify.error).mock.lastCall?.[1]?.action;
  expect(action).toMatchObject({label:"重新核对"});
  if(action && typeof action==="object" && "onClick" in action && typeof action.onClick==="function")
    await act(async()=>{(action.onClick as ()=>void)();});
  expect(host.querySelector('[data-session]')?.getAttribute("data-session")).toBe(oldId);
  expect(host.querySelector('[data-pending="false"]')).not.toBeNull();
  expect(create).not.toHaveBeenCalled();
});
it("does not treat a failed directory read as a deleted conversation", async () => {
  localStorage.setItem(selectedKey, oldId);
  ports.metadata.mockResolvedValue({id: oldId, title: "Saved", archived: false});
  ports.directory.mockRejectedValue("notFound");
  const {host} = await mount();
  expect(localStorage.getItem(selectedKey)).toBe(oldId);
  expect(localStorage.getItem(pendingKey)).toBeNull();
  expect(host.querySelector('[aria-label="重新核对对话"]')).toBeNull();
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("无法核对对话及上下文"),expect.objectContaining({action:expect.any(Object)}));
});
it("ignores a late missing-conversation response after an explicit new draft", async () => {
  localStorage.setItem(selectedKey, oldId);
  let reject!: (reason: string) => void;
  ports.metadata.mockReturnValue(new Promise((_, fail) => {reject = fail;}));
  const {host, render} = await mount();
  await render(1);
  const next = localStorage.getItem(selectedKey);
  await act(async () => reject("notFound"));
  expect(localStorage.getItem(selectedKey)).toBe(next);
  expect(host.querySelector('[data-session]')?.getAttribute("data-session")).toBe(next);
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("delivers an explanation to the new general draft only after leaving a directory conversation", async () => {
  localStorage.setItem(selectedKey, oldId);
  ports.metadata.mockResolvedValue({id:oldId, title:"Manuscript chat", archived:false});
  const {render} = await mount();
  const request = {text:"Explain selected PDF text", resolve:vi.fn(), reject:vi.fn()};
  await render(1, request);
  const next = localStorage.getItem(selectedKey);
  expect(next).not.toBe(oldId);
  expect(ports.deliveries).toHaveBeenCalledExactlyOnceWith(next, request.text);
});
