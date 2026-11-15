/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import {WritingAgentPanel} from "./WritingAgentPanel";
import type {WritingAgentMessage} from "./WritingAgentMessage";

const parsed = vi.hoisted(() => new Map<string, number>());
vi.mock("../../agent/markdown/compileMarkdown", async original => {
  const module = await original<typeof import("../../agent/markdown/compileMarkdown")>();
  return {...module, MarkdownCompiler: class extends module.MarkdownCompiler {
    override render(input: Parameters<InstanceType<typeof module.MarkdownCompiler>["render"]>[0]) {
      parsed.set(input.value, (parsed.get(input.value) ?? 0) + 1);
      return super.render(input);
    }
  }};
});

it.each([false, true])("treats an upward scroll as user intent even while the virtual window is restoring heights (running=%s)", async running => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const messages: WritingAgentMessage[] = [{id:"answer",role:"assistant",content:"当前回复"}];
  try {
    await act(async () => root.render(<WritingAgentPanel open isRunning={running} messages={messages}/>));
    const pane = host.querySelector<HTMLElement>(".writing-agent-messages")!;
    let height = 400;
    Object.defineProperties(pane,{clientHeight:{configurable:true,value:100},scrollHeight:{configurable:true,get:()=>height}});
    pane.scrollTop = 300;
    await act(async () => pane.dispatchEvent(new Event("scroll")));
    pane.dataset.transcriptAnchoring = "true";
    await act(async () => pane.dispatchEvent(new WheelEvent("wheel",{deltaY:-30,bubbles:true,cancelable:true})));
    expect(pane.dataset.followBottom).toBe("false");
    const position = pane.scrollTop;
    height = 500;
    await act(async () => root.render(<WritingAgentPanel open isRunning={running} messages={[...messages,{id:"next",role:"assistant",content:"新回复"}]}/>));
    expect(pane.scrollTop).toBe(position);
  } finally {await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();}
});

it("typing never reparses old Markdown and a stream delta only parses its changed message", async () => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const history: WritingAgentMessage[] = Array.from({length: 6}, (_, index) => ({id:`old-${index}`, turnId:`old-${index}`,
    role:"assistant", turnFinished:true, content:`历史 **${index}**，公式 \\(x_${index}\\)。`}));
  let messages: WritingAgentMessage[] = [...history, {id:"live", turnId:"live", role:"assistant", streaming:true, content:"新的内容"}];
  const render = () => <WritingAgentPanel open isRunning messages={messages}/>;
  try {
    await act(async () => root.render(render()));
    await act(async () => vi.advanceTimersByTimeAsync(160)); parsed.clear();
    const input = host.querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,"草稿");
      input.dispatchEvent(new Event("input", {bubbles:true}));
    });
    expect(input.value).toBe("草稿"); expect([...parsed.keys()]).toEqual([]);
    messages = [...history, {...messages.at(-1)!, content:"新的内容继续增加"}];
    await act(async () => root.render(render()));
    await act(async () => vi.advanceTimersByTimeAsync(80));
    expect([...parsed.keys()]).toEqual(["新的内容继续增加"]);
    expect(input.value).toBe("草稿");
  } finally {await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); vi.useRealTimers();}
});

it("remeasures delayed formula layout without reparsing history or pulling an older reader to the end", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const observers: {callback: () => void; targets: Element[]}[] = [];
  vi.stubGlobal("ResizeObserver", class {
    entry: typeof observers[number];
    constructor(callback: () => void) {this.entry = {callback, targets: []}; observers.push(this.entry);}
    observe = (target: Element) => {this.entry.targets.push(target);};
    disconnect = () => {};
  });
  const frames = new Map<number, FrameRequestCallback>(); let frameId = 0;
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => {frames.set(++frameId, callback); return frameId;});
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(id => {frames.delete(id);});
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<WritingAgentPanel open messages={[{id:"math", role:"assistant", content:String.raw`公式 \(E=mc^2\)`}]}/>));
    const pane = host.querySelector<HTMLElement>(".writing-agent-messages")!;
    const observer = observers.find(entry => entry.targets.includes(pane))!;
    let height = 400;
    Object.defineProperties(pane, {clientHeight:{value:100}, scrollHeight:{get:() => height}});
    const flush = () => {const pending = [...frames.values()]; frames.clear(); pending.forEach(callback => callback(0));};
    pane.scrollTop = 300; await act(async () => pane.dispatchEvent(new Event("scroll")));
    parsed.clear(); height = 450;
    await act(async () => {observer.callback(); flush();});
    expect(pane.scrollTop).toBe(350);
    await act(async () => pane.dispatchEvent(new WheelEvent("wheel", {deltaY:-40, bubbles:true, cancelable:true})));
    const readingPosition = pane.scrollTop; height = 500;
    await act(async () => {observer.callback(); flush();});
    expect(pane.scrollTop).toBe(readingPosition);
    expect([...parsed.keys()]).toEqual([]);
  } finally {await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();}
});
