/** @vitest-environment happy-dom */
import { act, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { AgentMessageSelectionAction } from "./AgentMessageSelectionAction";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(() => { window.getSelection()?.removeAllRanges(); vi.restoreAllMocks(); });

it("offers the same action for sent, assistant and pending bubbles, and clears stale selections", async () => {
  const added = vi.fn();
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  function Fixture({ enabled = true, scope = "one" }: { enabled?: boolean; scope?: string }) {
    const messages = useRef<HTMLDivElement>(null);
    const [draft, setDraft] = useState("现有草稿");
    return <><div ref={messages}>{["发送消息", "助手回答", "待处理消息"].map((text, i) =>
      <article key={text} data-agent-message-id={i}><div data-selectable-text><div className="writing-agent-markdown">{text}</div></div></article>)}</div>
      <AgentMessageSelectionAction root={messages} enabled={enabled} scope={scope} onAdd={(text, comment, options) => { added(text, comment, options); setDraft(`${draft}\n\n> ${text}`); }} />
      <output>{draft}</output></>;
  }
  async function select(index: number) {
    const text = host.querySelectorAll(".writing-agent-markdown")[index]!.firstChild!;
    const range = document.createRange(); range.selectNodeContents(text);
    range.getBoundingClientRect = () => new DOMRect(30, 80, 100, 20);
    await act(async () => { window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range); document.dispatchEvent(new Event("selectionchange")); });
  }
  try {
    await act(async () => root.render(<Fixture />));
    await act(async () => host.querySelector(".writing-agent-markdown")!.dispatchEvent(new PointerEvent("pointerdown", {button:0, bubbles:true})));
    await select(0);
    expect(document.querySelector(".agent-message-selection-action")).toBeNull();
    await act(async () => document.dispatchEvent(new PointerEvent("pointerup", {button:0, bubbles:true})));
    expect(document.querySelector(".agent-message-selection-action")?.textContent).toBe("添加批注");
    await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", {key:"ArrowRight", shiftKey:true, bubbles:true})));
    await select(0);
    expect(document.querySelector(".agent-message-selection-action")).toBeNull();
    await act(async () => document.dispatchEvent(new KeyboardEvent("keyup", {key:"Shift", shiftKey:false, bubbles:true})));
    expect(document.querySelector(".agent-message-selection-action")?.textContent).toBe("添加批注");
    for (const [index, text] of ["发送消息", "助手回答", "待处理消息"].entries()) {
      await select(index);
      expect(document.querySelector(".agent-message-selection-action")?.textContent).toBe("添加批注");
      await act(async () => document.querySelector<HTMLButtonElement>(".agent-message-selection-action button")!.click());
      expect(added).toHaveBeenCalledTimes(index);
      const textarea = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="批注 对话选段"]')!;
      if (index === 0) await act(async () => {Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "评论"); textarea.dispatchEvent(new Event("input", {bubbles: true}));});
      await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", {key:"Enter", bubbles:true})));
      expect(added).toHaveBeenLastCalledWith(text, index === 0 ? "评论" : "", {focusComposer: true});
    }
    expect(host.querySelector("output")?.textContent).toBe("现有草稿\n\n> 发送消息\n\n> 助手回答\n\n> 待处理消息");
    await select(0);
    await act(async () => window.dispatchEvent(new Event("scroll")));
    expect(document.querySelector(".agent-message-selection-action")).toBeNull();
    await select(0);
    await act(async () => { window.getSelection()?.removeAllRanges(); document.dispatchEvent(new Event("selectionchange")); });
    expect(document.querySelector(".agent-message-selection-action")).toBeNull();
    const bubbles = host.querySelectorAll(".writing-agent-markdown");
    const across = document.createRange();
    across.setStart(bubbles[0]!.firstChild!, 0); across.setEnd(bubbles[1]!.firstChild!, 2);
    await act(async () => { window.getSelection()!.addRange(across); document.dispatchEvent(new Event("selectionchange")); });
    expect(document.querySelector(".agent-message-selection-action")).toBeNull();
    await select(0);
    await act(async () => root.render(<Fixture scope="two" />));
    expect(document.querySelector(".agent-message-selection-action")).toBeNull();
    await select(0);
    await act(async () => root.render(<Fixture enabled={false} scope="two" />));
    expect(document.querySelector(".agent-message-selection-action")).toBeNull();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("preserves a frozen selection while commenting and adds it once on outside dismissal without stealing focus", async () => {
  const added = vi.fn();
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  function Fixture({scope = "one"}: {scope?: string}) {
    const messages = useRef<HTMLDivElement>(null);
    return <><div ref={messages}><article data-agent-message-id="one"><div data-selectable-text><div className="writing-agent-markdown">选中的文字</div></div></article></div>
      <AgentMessageSelectionAction root={messages} enabled scope={scope} onAdd={(text, comment, options) => added(scope, text, comment, options)}/><button type="button">外部按钮</button></>;
  }
  try {
    await act(async () => root.render(<Fixture />));
    const range = document.createRange(); range.selectNodeContents(host.querySelector(".writing-agent-markdown")!);
    range.getBoundingClientRect = () => new DOMRect(30, 80, 100, 20);
    await act(async () => {window.getSelection()!.addRange(range); document.dispatchEvent(new Event("selectionchange"));});
    await act(async () => document.querySelector<HTMLButtonElement>(".agent-message-selection-action button")!.click());
    const textarea = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="批注 对话选段"]')!;
    await act(async () => {Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "附加评论"); textarea.dispatchEvent(new Event("input", {bubbles: true}));});
    await act(async () => {window.getSelection()?.removeAllRanges(); document.dispatchEvent(new Event("selectionchange")); window.dispatchEvent(new Event("scroll"));});
    expect(document.querySelector('textarea[aria-label="批注 对话选段"]')).not.toBeNull();
    await act(async () => host.querySelector("button")!.dispatchEvent(new PointerEvent("pointerdown", {button:0, bubbles:true})));
    expect(added).toHaveBeenCalledTimes(1);
    expect(added).toHaveBeenCalledWith("one", "选中的文字", "附加评论", {focusComposer: false});
    expect(document.querySelector(".agent-message-selection-action")).toBeNull();
    await act(async () => {window.getSelection()!.addRange(range); document.dispatchEvent(new Event("selectionchange"));});
    await act(async () => document.querySelector<HTMLButtonElement>(".agent-message-selection-action button")!.click());
    const next = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="批注 对话选段"]')!;
    await act(async () => {Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(next, "旧会话草稿"); next.dispatchEvent(new Event("input", {bubbles: true}));});
    await act(async () => root.render(<Fixture scope="two" />));
    expect(added).toHaveBeenCalledTimes(1);
    expect(document.querySelector(".agent-message-selection-action")).toBeNull();
    await act(async () => {window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range); document.dispatchEvent(new Event("selectionchange"));});
    await act(async () => document.querySelector<HTMLButtonElement>(".agent-message-selection-action button")!.click());
    const afterSwitch = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="批注 对话选段"]')!;
    await act(async () => afterSwitch.dispatchEvent(new KeyboardEvent("keydown", {key:"Escape", bubbles:true})));
    expect(added).toHaveBeenCalledTimes(2);
    expect(added).toHaveBeenLastCalledWith("two", "选中的文字", "", {focusComposer: false});
    await act(async () => root.render(<Fixture scope="one" />));
    const restored = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="批注 对话选段"]')!;
    expect(restored.value).toBe("旧会话草稿");
    await act(async () => restored.dispatchEvent(new KeyboardEvent("keydown", {key:"Enter", bubbles:true})));
    expect(added).toHaveBeenLastCalledWith("one", "选中的文字", "旧会话草稿", {focusComposer: true});
  } finally {await act(async () => root.unmount()); host.remove();}
});

it("does not expose clipped text, hidden content, or message attachments as a quote", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  function Fixture() {
    const messages = useRef<HTMLDivElement>(null);
    return <><div ref={messages}><article data-agent-message-id="user"><div data-selectable-text>
      <div className="agent-user-message-text" data-collapsed="true"><p>折叠的文字</p></div>
      <div className="attachment">变更卡片</div>
    </div></article><article data-agent-message-id="assistant"><div data-selectable-text>
      <div className="writing-agent-markdown">可见<span hidden>隐藏文字</span></div>
    </div></article></div><AgentMessageSelectionAction root={messages} enabled scope="one" onAdd={() => {}} /></>;
  }
  async function select(element: Element) {
    const range = document.createRange(); range.selectNodeContents(element);
    range.getBoundingClientRect = () => new DOMRect(30, 80, 100, 20);
    await act(async () => { window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range); document.dispatchEvent(new Event("selectionchange")); });
  }
  try {
    await act(async () => root.render(<Fixture />));
    const collapsed = host.querySelector<HTMLElement>(".agent-user-message-text")!;
    Object.defineProperty(collapsed, "scrollHeight", { value: 200 });
    Object.defineProperty(collapsed, "clientHeight", { value: 100 });
    await select(collapsed.querySelector("p")!);
    expect(document.querySelector(".agent-message-selection-action")).toBeNull();
    await select(host.querySelector(".attachment")!);
    expect(document.querySelector(".agent-message-selection-action")).toBeNull();
    await select(host.querySelector(".writing-agent-markdown")!);
    expect(document.querySelector(".agent-message-selection-action")).toBeNull();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("restores an unfinished comment after its identified chat panel is unmounted and reopened", async () => {
  const added = vi.fn(), host = document.createElement("div"); document.body.append(host);
  let root = createRoot(host);
  function Fixture() {
    const messages = useRef<HTMLDivElement>(null);
    return <><div ref={messages}><article data-agent-message-id="message"><div data-selectable-text><div className="writing-agent-markdown">原选段</div></div></article></div>
      <AgentMessageSelectionAction root={messages} enabled scope='["conversation-restore",null,null]' onAdd={added}/></>;
  }
  try {
    await act(async () => root.render(<Fixture/>));
    const range = document.createRange(); range.selectNodeContents(host.querySelector(".writing-agent-markdown")!);
    range.getBoundingClientRect = () => new DOMRect(30, 80, 100, 20);
    await act(async () => {window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range); document.dispatchEvent(new Event("selectionchange"));});
    await act(async () => document.querySelector<HTMLButtonElement>(".agent-message-selection-action button")!.click());
    const editor = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="批注 对话选段"]')!;
    await act(async () => {Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(editor,"尚未提交"); editor.dispatchEvent(new Event("input", {bubbles:true}));});
    await act(async () => root.unmount());
    expect(added).not.toHaveBeenCalled();
    root = createRoot(host);
    await act(async () => root.render(<Fixture/>));
    const restored = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="批注 对话选段"]')!;
    expect(restored.value).toBe("尚未提交");
    await act(async () => restored.dispatchEvent(new KeyboardEvent("keydown", {key:"Enter", bubbles:true})));
    expect(added).toHaveBeenCalledWith("原选段", "尚未提交", {focusComposer:true});
  } finally {await act(async () => root.unmount()); host.remove();}
});

it("does not restore an unidentified new conversation into another panel for the same resource", async () => {
  const added = vi.fn(), host = document.createElement("div"); document.body.append(host);
  let root = createRoot(host);
  function Fixture() {
    const messages = useRef<HTMLDivElement>(null);
    return <><div ref={messages}><article data-agent-message-id="message"><div data-selectable-text><div className="writing-agent-markdown">旧选段</div></div></article></div>
      <AgentMessageSelectionAction root={messages} enabled scope='[null,"manuscript","same-resource"]' onAdd={added}/></>;
  }
  try {
    await act(async () => root.render(<Fixture/>));
    const range = document.createRange(); range.selectNodeContents(host.querySelector(".writing-agent-markdown")!);
    range.getBoundingClientRect = () => new DOMRect(30, 80, 100, 20);
    await act(async () => {window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range); document.dispatchEvent(new Event("selectionchange"));});
    await act(async () => document.querySelector<HTMLButtonElement>(".agent-message-selection-action button")!.click());
    const editor = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="批注 对话选段"]')!;
    await act(async () => {Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(editor,"旧草稿"); editor.dispatchEvent(new Event("input", {bubbles:true}));});
    await act(async () => root.unmount());
    root = createRoot(host);
    await act(async () => root.render(<Fixture/>));
    expect(document.querySelector('textarea[aria-label="批注 对话选段"]')).toBeNull();
    expect(added).not.toHaveBeenCalled();
  } finally {await act(async () => root.unmount()); host.remove();}
});

it("removes a successfully submitted pending comment even when the conversation changes during the request", async () => {
  let resolve!: () => void;
  const added = vi.fn(() => new Promise<void>(done => {resolve = done;}));
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  function Fixture({scope}: {scope:string}) {
    const messages = useRef<HTMLDivElement>(null);
    return <><div ref={messages}><article data-agent-message-id="message"><div data-selectable-text><div className="writing-agent-markdown">原选段</div></div></article></div>
      <AgentMessageSelectionAction root={messages} enabled scope={scope} onAdd={added}/></>;
  }
  const first = '["async-first",null,null]', second = '["async-second",null,null]';
  try {
    await act(async () => root.render(<Fixture scope={first}/>));
    const range = document.createRange(); range.selectNodeContents(host.querySelector(".writing-agent-markdown")!);
    range.getBoundingClientRect = () => new DOMRect(30, 80, 100, 20);
    await act(async () => {window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range); document.dispatchEvent(new Event("selectionchange"));});
    await act(async () => document.querySelector<HTMLButtonElement>(".agent-message-selection-action button")!.click());
    await act(async () => document.querySelector<HTMLTextAreaElement>('textarea[aria-label="批注 对话选段"]')!.dispatchEvent(new KeyboardEvent("keydown", {key:"Enter", bubbles:true})));
    await act(async () => root.render(<Fixture scope={second}/>));
    await act(async () => resolve());
    await act(async () => root.render(<Fixture scope={first}/>));
    expect(document.querySelector('textarea[aria-label="批注 对话选段"]')).toBeNull();
    expect(added).toHaveBeenCalledTimes(1);
  } finally {await act(async () => root.unmount()); host.remove();}
});
