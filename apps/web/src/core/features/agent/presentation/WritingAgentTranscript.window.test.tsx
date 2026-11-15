/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import {WritingAgentTranscript} from "./WritingAgentTranscript";
import type {WritingAgentMessage} from "./WritingAgentMessage";

it("mounts a bounded historical window, loads a distant search target and avoids offscreen attachment reads", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host), attachment = vi.fn(() => null);
  const messages: WritingAgentMessage[] = Array.from({length: 500}, (_, index) => ({id:`turn-${index}/answer`, turnId:`turn-${index}`,
    role:"assistant", turnFinished:true, content:`历史消息 ${index}`}));
  try {
    await act(async () => root.render(<WritingAgentTranscript messages={messages} isRunning={false} turnAttachment={attachment}/>));
    expect(host.querySelectorAll("article").length).toBeLessThanOrEqual(8);
    expect(host.textContent).toContain("历史消息 499"); expect(host.textContent).not.toContain("历史消息 100");
    expect(attachment.mock.calls.length).toBeLessThanOrEqual(8);
    attachment.mockClear();
    await act(async () => root.render(<WritingAgentTranscript messages={messages} isRunning={false} turnAttachment={attachment}
      searchTarget={{requestId:1, messageId:"turn-10/answer"}}/>));
    expect(host.querySelector('[data-agent-message-id="turn-10/answer"]')).not.toBeNull();
    expect(host.querySelectorAll("article").length).toBeLessThanOrEqual(9);
    expect(attachment.mock.calls.length).toBeLessThanOrEqual(9);
  } finally {await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();}
});

it("restores turn and tool disclosures after an offscreen turn is recycled", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const messages: WritingAgentMessage[] = [
    {id:"first/call",turnId:"first",role:"assistant",kind:"tool-call",toolName:"shell",toolCallId:"call",content:'{"cmd":"echo test"}',turnFinished:true},
    {id:"first/result",turnId:"first",role:"assistant",kind:"tool-result",toolName:"shell",toolCallId:"call",content:"test",turnFinished:true},
    {id:"first/final",turnId:"first",role:"assistant",phase:"finalAnswer",content:"首轮完成",turnFinished:true},
    ...Array.from({length:30},(_,index)=>({id:`later-${index}/answer`,turnId:`later-${index}`,role:"assistant" as const,content:`后续 ${index}`,turnFinished:true})),
  ];
  const render = (id: string, requestId: number) => root.render(<WritingAgentTranscript messages={messages} isRunning={false}
    searchTarget={{requestId,messageId:id}}/>);
  try {
    await act(async () => render("first/final",1));
    const first = host.querySelector('[data-transcript-row="first"]')!;
    const turn = first.querySelector<HTMLDetailsElement>(".writing-agent-turn-process")!;
    const tool = first.querySelector<HTMLDetailsElement>(".tool-history-row")!;
    await act(async () => {turn.open=true;turn.dispatchEvent(new Event("toggle"));tool.open=true;tool.dispatchEvent(new Event("toggle"));});
    await act(async () => render("later-29/answer",2));
    expect(host.querySelector('[data-transcript-row="first"]')).toBeNull();
    await act(async () => render("first/final",3));
    expect(host.querySelector<HTMLDetailsElement>('[data-transcript-row="first"] .writing-agent-turn-process')?.open).toBe(true);
    expect(host.querySelector<HTMLDetailsElement>('[data-transcript-row="first"] .tool-history-row')?.open).toBe(true);
  } finally {await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();}
});
