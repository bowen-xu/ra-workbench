// @vitest-environment happy-dom
import { act, memo, StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {selectAppStorage} from "../../runtime/appStorage";
import { AgentCanvasHost, AgentCanvasShell, useAgentCanvasMessage } from "./AgentCanvasShell";
import { useAgentDraftReferenceCount } from "./AgentDraftReferenceCount";
import { usePublishAgentActivity } from "./AgentActivity";
import { ActionMenu, ActionMenuItem } from "../../components/ActionMenu";
import { useAgentChatPin } from "./AgentChatPin";
import { TEXT_DOCUMENT_REVEALED } from "../../components/text-editor/textDocumentEvents";
import { defaultShortcuts } from "../settings/keyboardShortcuts";
import { contextDirectoryGateway } from "./context/contextDirectoryGateway";
import { OPEN_AGENT_CONVERSATION } from "./openAgentConversation";

vi.mock("./presentation/WritingAgentPanel", () => ({ WritingAgentPanel: () => null }));
vi.mock("./context/IndependentAgentWorkspace", () => ({ IndependentAgentWorkspace: ({open,workspaceActive,headerControls,drawer,onSelectContext,messageRequest}: {open:boolean;workspaceActive:boolean;headerControls:React.ReactNode;drawer:(content:React.ReactNode)=>React.ReactNode;onSelectContext:(id:string,directory:{manuscriptId:string;title:string;path:string},archived:boolean)=>void;messageRequest?:import("./AgentCanvasMessage").AgentCanvasMessage|null}) => {
  useEffect(()=>{messageRequest?.resolve();},[messageRequest]);
  return <section className="independent-test-panel" hidden={!open||workspaceActive}>{headerControls}<button onClick={()=>onSelectContext("conversation",{manuscriptId:"directory",title:"目录",path:"source"},false)}>选择配置了目录的对话</button><button onClick={()=>onSelectContext("conversation-2",{manuscriptId:"directory-2",title:"目录 2",path:"source"},false)}>选择另一个目录的对话</button>{drawer(<p>统一历史</p>)}</section>;
} }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let unmount: (() => Promise<void>) | undefined;
beforeEach(()=>{localStorage.clear();selectAppStorage(localStorage);});

it("does not rerender passive message senders when the chat opens or closes", async () => {
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host),rendered=vi.fn();
  unmount=async()=>{await act(async()=>root.unmount());host.remove();};
  const Passive=memo(function Passive(){rendered();useAgentCanvasMessage();return null;});
  await act(async()=>root.render(<AgentCanvasShell independent><Passive/></AgentCanvasShell>));
  rendered.mockClear();
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="打开对话"]')!.click());
  expect(rendered).not.toHaveBeenCalled();
});

it("preserves outside input focus when hiding the chat and retains its draft", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  unmount = async () => { await act(async () => root.unmount()); host.remove(); };
  function App(): React.JSX.Element {
    const [open, setOpen] = useState(true);
    return <AgentCanvasShell><input aria-label="外部输入" onFocus={() => setOpen(false)} />
      <AgentCanvasHost open={open} onOpenChange={setOpen} onNewConversation={() => undefined} conversations={null}>
        {() => <section className="writing-agent-panel" hidden={!open}><textarea defaultValue="保留草稿" /></section>}
      </AgentCanvasHost>
    </AgentCanvasShell>;
  }
  await act(async () => root.render(<App />));
  const draft = host.querySelector<HTMLTextAreaElement>('textarea')!;
  await act(async () => draft.focus());
  const outside = host.querySelector<HTMLInputElement>('[aria-label="外部输入"]')!;
  await act(async () => outside.focus());
  expect(document.activeElement).toBe(outside);
  expect(host.querySelector('.writing-agent-panel')?.hasAttribute('hidden')).toBe(true);
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="打开对话"]')?.click());
  expect(host.querySelector('textarea')).toBe(draft);
  expect(draft.value).toBe("保留草稿");
});
afterEach(async () => { await unmount?.(); unmount = undefined; });
it("opens the production independent host from a writing-first canvas without destroying the writing draft", async () => {
  const element = document.createElement("div"); document.body.append(element); const root = createRoot(element);
  unmount = async () => {await act(async () => root.unmount()); element.remove();};
  function App() {
    const [open, setOpen] = useState(true);
    return <AgentCanvasShell independent initialMode="writing" placement="contained"><AgentCanvasHost workspaceId="directory" open={open} onOpenChange={setOpen} onNewConversation={() => undefined} conversations={null}>
      {controls => <section className="writing-test-panel" hidden={!open}>{controls.headerControls}<textarea defaultValue="保留稿件草稿" /></section>}
    </AgentCanvasHost></AgentCanvasShell>;
  }
  await act(async () => root.render(<App />));
  const writing = element.querySelector<HTMLElement>(".writing-test-panel")!, draft = writing.querySelector<HTMLTextAreaElement>("textarea")!;
  expect(writing.hidden).toBe(false);
  await act(async () => writing.querySelector<HTMLButtonElement>('[aria-label="新建会话"]')!.click());
  expect(element.querySelector<HTMLElement>(".independent-test-panel")?.hidden).toBe(false);
  expect(element.querySelector(".writing-test-panel textarea")).toBe(draft); expect(draft.value).toBe("保留稿件草稿");
});
it("keeps a pinned workspace open after document navigation and preserves the pin across manual hide", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  unmount = async () => { await act(async () => root.unmount()); host.remove(); };
  function Controls() {
    const [pinned, setPinned] = useAgentChatPin();
    return <button aria-label="测试置顶" aria-pressed={pinned} onClick={() => setPinned(value => !value)} />;
  }
  await act(async () => root.render(<AgentCanvasShell independent defaultOpen><Controls /></AgentCanvasShell>));
  const pin = host.querySelector<HTMLButtonElement>('[aria-label="测试置顶"]')!;
  const panel = host.querySelector<HTMLElement>(".independent-test-panel")!;
  await act(async () => pin.click());
  await act(async () => window.dispatchEvent(new Event(TEXT_DOCUMENT_REVEALED)));
  expect(panel.hidden).toBe(false);
  await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", {...defaultShortcuts().toggleChat, bubbles: true, cancelable: true})));
  expect(panel.hidden).toBe(true);
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="打开对话"]')!.click());
  expect(panel.hidden).toBe(false);
  expect(pin.getAttribute("aria-pressed")).toBe("true");
  await act(async () => pin.click());
  await act(async () => window.dispatchEvent(new Event(TEXT_DOCUMENT_REVEALED)));
  expect(panel.hidden).toBe(true);
});
it("owns portaled host menus while leaving page menus outside the retained panel scope", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  unmount = async () => { await act(async () => root.unmount()); host.remove(); };
  function Host() {
    const [open, setOpen] = useState(true);
    return <AgentCanvasShell>
      <ActionMenu label="页面测试菜单" className="test-page-menu" trigger="页面" open><ActionMenuItem>页面操作</ActionMenuItem></ActionMenu>
      <AgentCanvasHost open={open} onOpenChange={setOpen} onNewConversation={() => undefined} conversations={null}>
        {() => <section hidden={!open}>
          <textarea defaultValue="保留草稿" />
          <ActionMenu label="聊天测试菜单" className="test-chat-menu" trigger="聊天" open><ActionMenuItem>聊天操作</ActionMenuItem></ActionMenu>
          <button onClick={() => setOpen(false)}>隐藏测试聊天</button>
        </section>}
      </AgentCanvasHost>
    </AgentCanvasShell>;
  }
  await act(async () => root.render(<StrictMode><Host /></StrictMode>));
  const panel = host.querySelector('[data-overlay-scope="panel"]')!;
  const menu = document.querySelector('[role="menu"][aria-label="聊天测试菜单"]')!;
  const draft = host.querySelector("textarea")!;
  expect(menu.closest("[data-overlay-scope]")).toBe(panel);
  expect(menu.closest("section")).toBeNull();
  expect(document.querySelector('[role="menu"][aria-label="页面测试菜单"]')?.closest("[data-overlay-scope]")).toBeNull();
  expect(host.closest("[inert]")).toBeNull();
  await act(async () => Array.from(host.querySelectorAll("button")).find(button => button.textContent === "隐藏测试聊天")!.click());
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="打开对话"]')!.click());
  expect(host.querySelector("textarea")).toBe(draft);
  expect(host.querySelector('[data-overlay-scope="panel"]')).toBe(panel);
  await act(async () => root.render(null));
  expect(document.querySelector('[role="menu"][aria-label="聊天测试菜单"]')).toBeNull();
});
it("keeps the collapsed launcher busy until every mounted activity ends or unmounts",async()=>{
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host),onOpenChange=vi.fn();
  unmount=async()=>{await act(async()=>root.unmount());host.remove();};
  function Activity({active}:{active:boolean}) {usePublishAgentActivity(active);return null;}
  const render=(first:boolean,second:boolean,showSecond=true)=>act(async()=>root.render(<StrictMode><AgentCanvasShell>
    <AgentCanvasHost open={false} onOpenChange={onOpenChange} onNewConversation={()=>undefined} conversations={null}>
      {()=> <><Activity active={first}/>{showSecond&&<Activity active={second}/>}</>}
    </AgentCanvasHost>
  </AgentCanvasShell></StrictMode>));
  await render(true,true);
  const launcher=host.querySelector<HTMLButtonElement>('[aria-label="打开对话"]')!;
  expect(launcher.getAttribute("aria-busy")).toBe("true");
  expect(launcher.getAttribute("aria-description")).toBe("正在处理消息");
  expect(launcher.querySelector(".agent-launcher-spinner")).not.toBeNull();
  await render(false,true);
  expect(launcher.getAttribute("aria-busy")).toBe("true");
  await act(async()=>launcher.click());
  expect(onOpenChange).toHaveBeenLastCalledWith(true);
  await render(false,true,false);
  expect(launcher.getAttribute("aria-busy")).toBe("false");
  expect(launcher.querySelector(".agent-launcher-spinner")).toBeNull();
  expect(launcher.querySelector(".lucide-message-circle")).not.toBeNull();
});
it("shows the writing draft reference count on the global launcher and updates after removal",async()=>{
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  unmount=async()=>{await act(async()=>root.unmount());host.remove();};
  function Composer(){const [evidence,setEvidence]=useState([{type:"annotation" as const,title:"原文",source:"source://notes/note:#block{b}",text:"quote",comment:""}]);useAgentDraftReferenceCount({text:"",bindings:[],evidence});return <button onClick={()=>setEvidence([])}>移除测试引用</button>;}
  await act(async()=>root.render(<AgentCanvasShell><AgentCanvasHost open={false} onOpenChange={()=>undefined} onNewConversation={()=>undefined} conversations={null}>{()=> <Composer/>}</AgentCanvasHost></AgentCanvasShell>));
  expect(host.querySelector('.writing-agent-trigger .agent-reference-count-badge')?.textContent).toBe("1");
  expect(host.querySelector('.writing-agent-trigger')?.getAttribute('aria-description')).toBe("草稿含 1 项引用");
  await act(async()=>[...host.querySelectorAll<HTMLButtonElement>('button')].find(button=>button.textContent==="移除测试引用")!.click());
  expect(host.querySelector('.writing-agent-trigger .agent-reference-count-badge')).toBeNull();
});
it("keeps silent draft captures hidden while ordinary Add reveals the same chat",async()=>{
  const {useAgentCanvasMessage}=await import("./AgentCanvasShell");let send!:ReturnType<typeof useAgentCanvasMessage>;
  function Sender(){send=useAgentCanvasMessage();return null;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  unmount=async()=>{await act(async()=>root.unmount());host.remove();};
  await act(async()=>root.render(<AgentCanvasShell independent><Sender/></AgentCanvasShell>));
  const evidence=[{type:"annotation" as const,title:"PDF",source:"source://library/paper/pdf",text:"原文",comment:"评论"}];
  let receipt!:Promise<void>;
  await act(async()=>{receipt=send("",undefined,{mode:"draft",reveal:false,evidence});});await receipt;
  expect(host.querySelector<HTMLElement>(".independent-test-panel")?.hidden).toBe(true);
  await act(async()=>{receipt=send("",undefined,{mode:"draft",evidence});});await receipt;
  expect(host.querySelector<HTMLElement>(".independent-test-panel")?.hidden).toBe(false);
});

it("contains the chat, history and hidden launcher in an embedded presentation frame", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const newConversation = vi.fn();
  unmount = async () => { await act(async () => root.unmount()); host.remove(); };
  function Embedded(): React.JSX.Element {
    const [open, setOpen] = useState(true);
    return <main><section aria-label="展示区域"><AgentCanvasShell placement="contained">
      <AgentCanvasHost open={open} onOpenChange={setOpen} onNewConversation={newConversation} conversations={<p>保留历史</p>}>
        {controls => <section className="writing-agent-panel" hidden={!open}>{controls.headerControls}
          <button onClick={() => setOpen(false)}>收起</button><textarea defaultValue="保留草稿" />
        </section>}
      </AgentCanvasHost>
    </AgentCanvasShell></section><footer>预览控制</footer></main>;
  }
  await act(async () => root.render(<Embedded />));
  const frame = host.querySelector('[aria-label="展示区域"]');
  const layer = frame?.querySelector('.agent-canvas-shell[data-placement="contained"] > .agent-canvas-layer');
  const panel = layer?.querySelector('.writing-agent-panel');
  const draft = panel?.querySelector('textarea');
  expect([...panel!.querySelectorAll('button[aria-label]')].map(button => button.getAttribute('aria-label'))).toEqual(["新建会话", "对话列表"]);
  expect(panel?.querySelector('.agent-canvas-header-divider')).not.toBeNull();
  await act(async () => panel?.querySelector<HTMLButtonElement>('[aria-label="新建会话"]')?.click());
  expect(newConversation).toHaveBeenCalledOnce();
  await act(async () => (panel?.querySelector('[aria-label="对话列表"]') as HTMLButtonElement).click());
  expect(layer?.querySelector('[aria-label="对话列表面板"]')?.textContent).toContain("保留历史");
  await act(async () => (panel?.querySelector('button:not([aria-label])') as HTMLButtonElement).click());
  const launcher = layer?.querySelector<HTMLButtonElement>('[aria-label="打开对话"]');
  expect(launcher).not.toBeNull();
  await act(async () => launcher?.click());
  expect(layer?.querySelector('textarea')).toBe(draft);
  expect(frame?.querySelectorAll('.agent-canvas-layer')).toHaveLength(1);
  expect(host.querySelector('footer .agent-canvas-layer')).toBeNull();
});

it("keeps one application launcher and retains the runtime while the resource page is hidden", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); const mounted = vi.fn(), disposed = vi.fn();
  unmount = async () => { await act(async () => root.unmount()); host.remove(); };
  function Runtime({ header, open, close }: { header: React.ReactNode; open: boolean; close: () => void }): React.JSX.Element {
    const [text, setText] = useState("草稿");
    useEffect(() => { mounted(); return disposed; }, []);
    return <section className="writing-agent-panel" hidden={!open}>{header}<button type="button" onClick={close}>收起</button>
      <input aria-label="测试草稿" value={text} onChange={event => setText(event.target.value)} />
    </section>;
  }
  function Resource(): React.JSX.Element {
    const [open, setOpen] = useState(false);
    return <AgentCanvasHost open={open} onOpenChange={setOpen} onNewConversation={vi.fn()} conversations={<button type="button">历史对话</button>}>
      {controls => <Runtime header={controls.headerControls} open={open} close={() => setOpen(false)} />}
    </AgentCanvasHost>;
  }
  function App(): React.JSX.Element {
    const [page, setPage] = useState("writing");
    return <AgentCanvasShell><button onClick={() => setPage("library")}>文献库</button><div hidden={page !== "writing"}><Resource /></div></AgentCanvasShell>;
  }
  await act(async () => root.render(<App />));
  expect(host.querySelector('.agent-canvas-shell')?.getAttribute('data-placement')).toBe("viewport");
  const click = async (label: string): Promise<void> => { await act(async () => {
    const button = [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.getAttribute("aria-label") === label || node.textContent === label);
    if (!button) throw Error(label); button.click();
  }); };
  expect(host.querySelectorAll('[aria-label="打开对话"]')).toHaveLength(1);
  expect(host.querySelector('[aria-label="对话列表面板"]')).toBeNull();
  await click("打开对话");
  const input = host.querySelector('input[aria-label="测试草稿"]');
  await click("对话列表");
  const drawer = host.querySelector<HTMLElement>('[aria-label="对话列表面板"]');
  expect(drawer?.hidden).toBe(false);
  expect(drawer?.textContent).toContain("历史对话");
  await click("文献库");
  expect(host.querySelector('input[aria-label="测试草稿"]')).toBe(input);
  expect(mounted).toHaveBeenCalledTimes(1); expect(disposed).not.toHaveBeenCalled();
  await act(async () => input?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(drawer?.hidden).toBe(false);
  await click("隐藏对话列表");
  expect(drawer?.hidden).toBe(true);
  expect(document.activeElement?.getAttribute("aria-label")).toBe("对话列表");
  await click("收起");
  expect(document.activeElement?.getAttribute("aria-label")).toBe("打开对话");
  await click("打开对话");
  expect(host.querySelector('input[aria-label="测试草稿"]')).toBe(input);
  expect(drawer?.hidden).toBe(true);
});
it("selects a project conversation without opening a project or selecting an editor host", async () => {
  const element=document.createElement("div");document.body.append(element);const root=createRoot(element);
  const select=vi.fn();
  unmount=async()=>{await act(async()=>root.unmount());element.remove();};
  await act(async()=>root.render(<AgentCanvasShell independent defaultOpen>
    <AgentCanvasHost workspaceId="directory" selectedId="old" onSelect={select} open={false} onOpenChange={()=>undefined} onNewConversation={()=>undefined} conversations={null}>{()=>null}</AgentCanvasHost>
  </AgentCanvasShell>));
  await act(async()=>[...element.querySelectorAll("button")].find(button=>button.textContent==="选择配置了目录的对话")!.click());
  expect(select).not.toHaveBeenCalled();
  expect(element.querySelector<HTMLElement>(".independent-test-panel")?.hidden).toBe(false);
});

it("opens an external history conversation without consulting or navigating its directory", async () => {
  const element=document.createElement("div");document.body.append(element);const root=createRoot(element);
  unmount=async()=>{await act(async()=>root.unmount());element.remove();vi.restoreAllMocks();};
  const directory=vi.spyOn(contextDirectoryGateway,"read"),resolve=vi.fn();
  await act(async()=>root.render(<AgentCanvasShell independent>{null}</AgentCanvasShell>));
  await act(async()=>window.dispatchEvent(new CustomEvent(OPEN_AGENT_CONVERSATION,{detail:{id:"history",resolve,reject:vi.fn()},cancelable:true})));
  expect(resolve).toHaveBeenCalledOnce();expect(directory).not.toHaveBeenCalled();
  expect(element.querySelector<HTMLElement>(".independent-test-panel")?.hidden).toBe(false);
});

it("retains inactive resource hosts without letting their updates replace the active host", async () => {
  const element = document.createElement("div"); document.body.append(element); const root = createRoot(element);
  unmount = async () => { await act(async () => root.unmount()); element.remove(); };
  const mounted = vi.fn(), disposed = vi.fn(), observed = vi.fn();
  function Runtime({name}: {name: string}): React.JSX.Element {
    useEffect(() => {
      mounted(name);
      const handle = (): void => observed(name);
      window.addEventListener("test-host-event", handle);
      return () => {window.removeEventListener("test-host-event", handle); disposed(name);};
    }, [name]);
    return <input aria-label={`草稿 ${name}`} defaultValue={`保留 ${name}`}/>;
  }
  function App(): React.JSX.Element {
    const [active, setActive] = useState("A"), [openA, setOpenA] = useState(true), [openB, setOpenB] = useState(true);
    const [selectedB, setSelectedB] = useState("B-old");
    return <AgentCanvasShell independent initialMode="writing">
      <button onClick={() => setActive("B")}>切到 B 宿主</button>
      <button onClick={() => {setOpenB(false); setSelectedB("B-new");}}>更新后台 B</button>
      <button onClick={() => setOpenB(true)}>重新打开后台 B</button>
      <AgentCanvasHost enabled={active === "A"} workspaceId="A" selectedId="A" open={openA} onOpenChange={setOpenA} onNewConversation={() => undefined} conversations={null}>
        {() => <Runtime name="A"/>}
      </AgentCanvasHost>
      <AgentCanvasHost enabled={active === "B"} workspaceId="B" selectedId={selectedB} open={openB} onOpenChange={setOpenB} onNewConversation={() => undefined} conversations={null}>
        {() => <Runtime name="B"/>}
      </AgentCanvasHost>
    </AgentCanvasShell>;
  }
  await act(async () => root.render(<App/>));
  const first = element.querySelector<HTMLInputElement>('[aria-label="草稿 A"]')!, second = element.querySelector<HTMLInputElement>('[aria-label="草稿 B"]')!;
  expect(first.closest<HTMLElement>(".agent-canvas-host")?.hidden).toBe(false);
  expect(second.closest<HTMLElement>(".agent-canvas-host")?.hidden).toBe(true);
  async function click(text: string): Promise<void> {
    const button = [...element.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === text);
    expect(button).toBeDefined(); await act(async () => button!.click());
  }
  await click("更新后台 B");
  expect(first.closest<HTMLElement>(".agent-canvas-host")?.hidden).toBe(false);
  expect(element.querySelector('[aria-label="打开对话"]')).toBeNull();
  await click("重新打开后台 B");
  expect(first.closest<HTMLElement>(".agent-canvas-host")?.hidden).toBe(false);
  await click("切到 B 宿主");
  expect(second.closest<HTMLElement>(".agent-canvas-host")?.hidden).toBe(false);
  expect(first.closest<HTMLElement>(".agent-canvas-host")?.hidden).toBe(true);
  expect(first.value).toBe("保留 A"); expect(second.value).toBe("保留 B");
  expect(mounted.mock.calls.map(call => call[0])).toEqual(["A", "B"]);
  await act(async () => window.dispatchEvent(new Event("test-host-event")));
  expect(observed.mock.calls.map(call => call[0])).toEqual(["A", "B"]);
  expect(disposed).not.toHaveBeenCalled();
});

it("submits a reader action into the current writing conversation without creating a new conversation or changing its draft", async () => {
  const {useAgentCanvasMessage,useAgentCanvasExternalSubmission}=await import("./AgentCanvasShell");
  const receive=vi.fn(async()=>undefined),create=vi.fn();let send!:ReturnType<typeof useAgentCanvasMessage>;
  function Sender(){send=useAgentCanvasMessage();return null;}
  function Runtime(){useAgentCanvasExternalSubmission(receive);return <input aria-label="current draft" defaultValue="retained draft"/>;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  unmount=async()=>{await act(async()=>root.unmount());host.remove();};
  await act(async()=>root.render(<AgentCanvasShell><Sender/><AgentCanvasHost selectedId="current-conversation" open onOpenChange={()=>undefined} onNewConversation={create} conversations={null}>{()=> <Runtime/>}</AgentCanvasHost></AgentCanvasShell>));
  const resources=[{kind:"paper" as const,resourceId:"paper",attachmentId:"pdf"}];
  await act(async()=>send("explain selection",resources));
  expect(receive).toHaveBeenCalledExactlyOnceWith("explain selection",resources);expect(create).not.toHaveBeenCalled();expect(document.querySelector<HTMLInputElement>('[aria-label="current draft"]')!.value).toBe("retained draft");
});
