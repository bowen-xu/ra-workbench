// @vitest-environment happy-dom
import {act, useRef, useState} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {AgentComposerTextInput, type AgentComposerInputHandle} from "./AgentComposerTextInput";
import {AgentMarkdown} from "./AgentMarkdown";
import {MarkdownRenderScope} from "./markdown/MarkdownRenderScope";
import type {MarkdownWorkerPort} from "./markdown/MarkdownRenderQueue";
const workers = vi.hoisted(() => [] as MarkdownWorkerPort[]);
vi.mock("./markdown/createMarkdownWorker", () => ({createMarkdownWorker: () => {
  const port: MarkdownWorkerPort = {onmessage: null, onerror: null, onmessageerror: null,
    postMessage: vi.fn(), terminate: vi.fn()};
  workers.push(port); return port;
}}));
afterEach(() => {workers.length = 0; vi.unstubAllGlobals();});
it("keeps typing, composition, selection and the latest host callbacks independent of a busy output worker", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host), changed = vi.fn();
  function Harness({output, version}: {output: string; version: number}) {
    const [text, setText] = useState("");
    const input = useRef<AgentComposerInputHandle>(null);
    return <><MarkdownRenderScope><AgentMarkdown value={output} streaming/></MarkdownRenderScope>
      <AgentComposerTextInput inputRef={input} aria-label="输入" draft={{text, bindings: []}}
        onChange={value => {changed(version, value); setText(value);}} onDraftChange={draft => setText(draft.text)}
        onCaretChange={() => undefined} onKeyDown={() => undefined}/></>;
  }
  try {
    await act(async () => root.render(<Harness output="长回复" version={1}/>));
    const input = host.querySelector("textarea")!;
    input.focus(); input.dispatchEvent(new CompositionEvent("compositionstart", {bubbles: true}));
    function type(value: string): void {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, value);
      input.dispatchEvent(new Event("input", {bubbles: true}));
    }
    await act(async () => type("zhong"));
    input.setSelectionRange(2, 4);
    // No Worker reply is delivered: output is genuinely pending throughout.
    for (let i = 2; i <= 20; i++) await act(async () => root.render(<Harness output={`长回复${i}`} version={i}/>));
    expect(host.querySelector("textarea")).toBe(input);
    expect(document.activeElement).toBe(input);
    expect([input.selectionStart, input.selectionEnd]).toEqual([2, 4]);
    expect(input.value).toBe("zhong");
    await act(async () => {
      type("中文"); input.dispatchEvent(new CompositionEvent("compositionend", {bubbles: true, data: "中文"}));
    });
    expect(input.value).toBe("中文");
    expect(changed.mock.lastCall).toEqual([20, "中文"]);
    expect(workers).toHaveLength(1);
    expect(workers[0]!.postMessage).toHaveBeenCalledOnce();
  } finally {await act(async () => root.unmount()); host.remove();}
  expect(workers[0]!.terminate).toHaveBeenCalledOnce();
});
