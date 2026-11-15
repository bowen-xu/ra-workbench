/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import {useConversationDraft} from "./AgentDraftProvider";
import {ConversationDraftStore, conversationDraftKey} from "./conversationDraftStore";
import {WritingAgentPanel} from "./presentation/WritingAgentPanel";

it("subscribes the composer to text without rerendering its execution host and submits the current draft", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host), store = new ConversationDraftStore(), submitted = vi.fn();
  const scope = {hostKind:"manuscript" as const, hostId:"m", conversationId:"c"};
  let renders = 0;
  function Runtime() {
    renders++;
    const [, setText, , resources] = useConversationDraft(store, scope, false);
    return <WritingAgentPanel open draftResources={resources} onDraftChange={setText} onSubmit={submitted}/>;
  }
  try {
    await act(async () => root.render(<Runtime/>)); const before = renders;
    const input = host.querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,"当前新草稿");
      input.dispatchEvent(new Event("input",{bubbles:true}));
    });
    expect(input.value).toBe("当前新草稿"); expect(renders).toBe(before);
    expect(store.read(conversationDraftKey(scope))).toBe("当前新草稿");
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="发送消息"]')!.click());
    expect(submitted.mock.calls[0]?.[0]).toBe("当前新草稿");
    await act(async () => store.update(conversationDraftKey(scope),"恢复的草稿"));
    expect(input.value).toBe("恢复的草稿"); expect(renders).toBe(before);
    await act(async () => store.updateStructured(conversationDraftKey(scope), {text:"恢复的草稿",bindings:[],
      evidence:[{type:"annotation",source:"source://notes/note:#block{b}",title:"原文",text:"quote",comment:""}]}));
    expect(host.querySelector(".agent-evidence-disclosure")?.textContent).toBe("1 条批注");
    expect(renders).toBe(before);
  } finally {await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();}
});
