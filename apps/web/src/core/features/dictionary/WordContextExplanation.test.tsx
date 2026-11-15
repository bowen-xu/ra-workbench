/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { WordExplanationGateway, WordFollowUpReply } from "@research-assistant/core/dictionary";
import { WordContextExplanation } from "./WordContextExplanation";
import { notify } from "../../services/notifications";

let host: HTMLDivElement, root: ReturnType<typeof createRoot>;
const context = { word: "work", before: "In this ", after: ", we study vision." };
const explanation = { translation: "本研究", explanation: "指本文的研究工作。", providerId: "test", modelId: "test" };
beforeEach(() => { vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });
async function render(gateway: WordExplanationGateway<AbortSignal>) { await act(async () => root.render(<StrictMode><WordContextExplanation explanation={explanation} context={context} gateway={gateway} /></StrictMode>)); }
async function click(label: string) { await act(async () => host.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)?.click()); }
async function question(text: string) { await act(async () => {
  const input = host.querySelector("textarea"); if (!input) throw new Error("No question input");
  Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")?.set?.call(input,text); input.dispatchEvent(new Event("input", { bubbles:true }));
}); }
it("shows two separate meaning paragraphs without model footer and sends only the local history", async () => {
  const followUp = vi.fn<WordExplanationGateway<AbortSignal>["followUp"]>(async () => ({ text: "因为 this work 指代当前研究。", providerId: "test", modelId: "test" }));
  const explain = vi.fn(); await render({ explain, followUp });
  expect(host.querySelector(".dictionary-context-translation")?.textContent).toBe("本研究");
  expect(host.querySelector(".dictionary-context-reason")?.textContent).toBe("指本文的研究工作。");
  expect(host.querySelector("small")).toBeNull(); expect(host.querySelector("textarea")).toBeNull();
  await click("继续追问词义"); await question("为什么不是劳动？"); await click("发送追问");
  expect(followUp).toHaveBeenCalledWith({ context, explanation: { translation:"本研究", explanation:"指本文的研究工作。" }, messages:[{role:"user",text:"为什么不是劳动？"}] },expect.any(AbortSignal));
  expect(host.textContent).toContain("因为 this work"); expect(host.querySelector("textarea")?.value).toBe(""); expect(explain).not.toHaveBeenCalled();
  await question("this 是什么意思？"); await click("发送追问");
  expect(followUp.mock.calls[1]?.[0].messages).toHaveLength(3);
});
it("keeps failed questions for retry and cancels when the local chat closes", async () => {
  const error = vi.spyOn(notify,"error").mockImplementation(()=>"toast");
  let signal: AbortSignal | undefined, finish!: (result: WordFollowUpReply)=>void;
  const followUp = vi.fn<WordExplanationGateway<AbortSignal>["followUp"]>().mockRejectedValueOnce(new Error("Offline"))
    .mockImplementation((_draft,inputSignal) => { signal=inputSignal; return new Promise(resolve=> { finish=resolve; }); });
  await render({explain:vi.fn(),followUp}); await click("继续追问词义"); await question("为什么？"); await click("发送追问");
  expect(error).toHaveBeenCalledWith("Offline"); expect(host.querySelector("textarea")?.value).toBe("为什么？");
  await click("发送追问"); await click("收起局部追问"); expect(signal?.aborted).toBe(true);
  await act(async()=>finish({text:"Late",providerId:"test",modelId:"test"}));
  await click("继续追问词义"); expect(host.textContent).not.toContain("Late"); expect(host.querySelector("textarea")?.value).toBe("为什么？");
});
