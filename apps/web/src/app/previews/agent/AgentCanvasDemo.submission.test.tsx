/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentCanvasDemo } from "./AgentCanvasDemo";
import { getAppStorage, selectAppStorage } from "../../../core/runtime/appStorage";
import { notify } from "../../../core/services/notifications";

const created = vi.hoisted(() => ({states: [] as unknown[]}));
type SubmissionState = InstanceType<typeof import("./WritingDemoSubmissionStates")["WritingDemoSubmissionStates"]>;
vi.mock("./WritingDemoSubmissionStates", async importOriginal => {
  const actual = await importOriginal<typeof import("./WritingDemoSubmissionStates")>();
  return {...actual, WritingDemoSubmissionStates: class extends actual.WritingDemoSubmissionStates {
    constructor(...args: ConstructorParameters<typeof actual.WritingDemoSubmissionStates>) {super(...args); created.states.push(this);}
  }};
});

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
beforeEach(() => {
  created.states.length = 0;
  vi.spyOn(notify, "error");
  const values = new Map<string, string>();
  selectAppStorage({get length(){return values.size;},key:index=>[...values.keys()][index]??null,
    getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},
    removeItem:key=>{values.delete(key);},clear:()=>values.clear()});
});
afterEach(() => vi.restoreAllMocks());

for (const [scenario, status, action] of [
  ["submissionPreparing", null, "继续模拟发送"],
  ["submissionCheckingFailed", null, null],
  ["submissionPrepareFailed", null, null],
  ["submissionStartFailed", null, null],
] as const) {
  it(`previews ${scenario} through the production send UI and recovers to a running turn`, async () => {
    const host = document.createElement("div"); document.body.append(host);
    const root = createRoot(host);
    const button = (text: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === text)!;
    try {
      await act(async () => root.render(<AgentCanvasDemo />));
      await vi.waitFor(async () => {
        await act(async () => Promise.resolve());
        expect(host.querySelector('[data-action="stop"]')).not.toBeNull();
      });
      await act(async () => {
        const select = host.querySelector<HTMLSelectElement>('[aria-label="模拟消息场景"]')!;
        select.value = scenario;
        select.dispatchEvent(new Event("change", { bubbles: true }));
      });
      await vi.waitFor(async () => {
        await act(async () => Promise.resolve());
        expect(host.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(false);
        expect(host.querySelector('[data-action="stop"]')).toBeNull();
      });
      const submission = (created.states.at(-1) as SubmissionState).gateways.submission;
      const prepare = vi.spyOn(submission, "prepareWriting"), start = vi.spyOn(submission, "startWriting");
      if (scenario === "submissionPreparing") await act(async () => button("继续模拟发送").click());
      await act(async () => {
        const input = host.querySelector<HTMLTextAreaElement>("textarea")!;
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, "检查发送状态");
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await vi.waitFor(() => expect(host.querySelector<HTMLButtonElement>('[data-action="send"]')?.disabled).toBe(false));
      await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')!.click());
      expect(host.querySelector('[aria-label="写作提交状态"]')).toBeNull();
      if (scenario === "submissionPreparing" || scenario === "submissionCheckingFailed") {
        await vi.waitFor(async () => { await act(async () => Promise.resolve()); expect(host.querySelector('[aria-label="输入框准备状态"]')?.textContent).toContain("正在核对请求"); });
        expect(host.querySelector('.writing-agent-messages [aria-label="输入框准备状态"]')).toBeNull();
        expect(host.querySelector('.agent-composer-input-state [aria-label="输入框准备状态"]')).not.toBeNull();
        expect(host.querySelector('[data-action="cancel-send"]')).not.toBeNull();
        expect(host.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(true);
        expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("检查发送状态");
      }
      if (scenario === "submissionCheckingFailed") await act(async () => button("模拟核对失败").click());
      if (scenario === "submissionCheckingFailed" || scenario === "submissionPrepareFailed" || scenario === "submissionStartFailed") {
        if (scenario === "submissionStartFailed") {
          await vi.waitFor(() => expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("草稿已保留")));
          await vi.waitFor(() => expect(host.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(false));
          expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("检查发送状态");
          expect(host.querySelector('[aria-label="发送恢复"]')).toBeNull();
          expect(host.querySelector('[data-action="stop"]')).toBeNull();
          expect(host.querySelector('.writing-agent-message.user:not([data-pending-submission])')).toBeNull();
          const originalRequest = prepare.mock.calls[0]![0].requestId;
          expect(prepare).toHaveBeenCalledOnce(); expect(start).toHaveBeenCalledOnce();
          expect(Array.from({length: getAppStorage().length}, (_, index) => getAppStorage().key(index)!).filter(key => key.startsWith("research-assistant:writing-prepare:"))).toHaveLength(0);
          await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')!.click());
          await vi.waitFor(() => expect(start).toHaveBeenCalledTimes(2));
          expect(prepare).toHaveBeenCalledTimes(2);
          expect(prepare.mock.calls[1]![0].requestId).not.toBe(originalRequest);
          expect(start.mock.calls[1]![0].requestId).toBe(prepare.mock.calls[1]![0].requestId);
        }
        else {
          await vi.waitFor(() => expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("完整请求已保留"), expect.objectContaining({dismissible: false})));
          expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("检查发送状态");
          expect([...host.querySelectorAll(".writing-agent-message.user:not([data-pending-submission])")].map(node => node.textContent).join("\n")).not.toContain("检查发送状态");
          expect(host.querySelector('[data-pending-submission="true"]')?.textContent).toContain("检查发送状态");
          expect(start).not.toHaveBeenCalled();
          const storage = getAppStorage(), locator = Array.from({length: storage.length}, (_, index) => storage.key(index)!).find(key => key.startsWith("research-assistant:writing-prepare:"))!;
          const original = JSON.parse(storage.getItem(locator)!).intent;
          const actions = () => vi.mocked(notify.error).mock.calls.at(-1)?.[1]?.actions;
          await act(async () => actions()!.find(action => action.label === "核对原准备")!.onClick());
          expect(JSON.parse(storage.getItem(locator)!).intent).toEqual(original);
          expect(host.querySelector('[data-action="stop"]')).toBeNull();
          expect(host.querySelector('.writing-agent-message.user:not([data-pending-submission])')?.textContent ?? "").not.toContain("检查发送状态");
          await act(async () => actions()!.find(action => action.label === "重试原准备")!.onClick());
          expect(JSON.parse(storage.getItem(locator)!).intent).toEqual(original);
          expect(host.querySelector('[data-action="stop"]')).toBeNull();
          await vi.waitFor(() => expect(actions()?.find(action => action.label === "重新发送")).toBeDefined());
          await act(async () => actions()!.find(action => action.label === "重新发送")!.onClick());
        }
      }
      if (status) await vi.waitFor(async () => { await act(async () => Promise.resolve()); expect(host.textContent).toContain(status); });
      else expect(host.querySelector('[aria-label="发送恢复"]')).toBeNull();
      expect(host.textContent).not.toContain("发送内容预览");
      expect(host.querySelector('[aria-label="发送恢复"]')?.textContent ?? "").not.toContain("检查发送状态");
      if (action) await act(async () => button(action).click());
      await vi.waitFor(() => expect(host.querySelector('[data-action="stop"]')).not.toBeNull());
      expect(host.querySelector('[aria-label="写作提交状态"]')).toBeNull();
      expect(host.querySelector('[aria-label="发送恢复"]')).toBeNull();
      expect(host.querySelector('[aria-label="输入框准备状态"]')).toBeNull();
      expect(host.querySelector('.writing-agent-message.user:not([data-pending-submission])')?.textContent).toContain("检查发送状态");
      if (scenario === "submissionPreparing") {
        await act(async () => button("继续模拟发送").click());
        await act(async () => host.querySelector<HTMLButtonElement>('[data-action="stop"]')!.click());
        await vi.waitFor(async () => {
          await act(async () => Promise.resolve());
          expect(host.querySelector('[data-action="stop"]')).toBeNull();
          expect(host.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(false);
        });
        await act(async () => {
          const input = host.querySelector<HTMLTextAreaElement>("textarea")!;
          Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, "再次检查发送状态");
          input.dispatchEvent(new Event("input", { bubbles: true }));
        });
        await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')!.click());
        await vi.waitFor(async () => {
          await act(async () => Promise.resolve());
          expect(host.querySelector('[aria-label="输入框准备状态"]')?.textContent).toContain("正在核对请求");
        });
        expect(host.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(true);
        expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("再次检查发送状态");
        expect(host.querySelector('.writing-agent-message.user:not([data-pending-submission])')?.textContent).not.toContain("再次检查发送状态");
        await act(async () => button("继续模拟发送").click());
        await vi.waitFor(() => expect(host.querySelector('[data-action="stop"]')).not.toBeNull());
        expect(host.querySelector('[aria-label="输入框准备状态"]')).toBeNull();
        expect([...host.querySelectorAll(".writing-agent-message.user:not([data-pending-submission])")].at(-1)?.textContent).toContain("再次检查发送状态");
      }
    } finally {
      await act(async () => root.unmount()); host.remove();
    }
  });
}

it("preserves a pending request across scene switches without sending it or showing a late error", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const select = async (value: string) => act(async () => {
    const control = host.querySelector<HTMLSelectElement>('[aria-label="模拟消息场景"]')!;
    control.value = value; control.dispatchEvent(new Event("change", { bubbles: true }));
  });
  try {
    await act(async () => root.render(<AgentCanvasDemo />));
    await vi.waitFor(async () => { await act(async () => Promise.resolve()); expect(host.querySelector('[data-action="stop"]')).not.toBeNull(); });
    await select("submissionPreparing");
    await vi.waitFor(async () => { await act(async () => Promise.resolve()); expect(host.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(false); });
    const originalSubmission = (created.states.at(-1) as SubmissionState).gateways.submission;
    const originalPrepare = vi.spyOn(originalSubmission, "prepareWriting"), originalStart = vi.spyOn(originalSubmission, "startWriting");
    await act(async () => {
      const input = host.querySelector<HTMLTextAreaElement>("textarea")!;
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, "不要发送旧场景");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')!.click());
    await vi.waitFor(() => expect(originalPrepare).toHaveBeenCalledOnce());
    const original = originalPrepare.mock.calls[0]![0];
    expect(host.querySelector('[aria-label="写作提交状态"]')).toBeNull();
    await select("submissionStartFailed");
    await vi.waitFor(async () => {
      await act(async () => Promise.resolve());
      expect((created.states.at(-1) as SubmissionState).scenario).toBe("submissionStartFailed");
    });
    expect(host.querySelector('.writing-agent-message.user:not([data-pending-submission])')?.textContent ?? "").not.toContain("不要发送旧场景");
    const storage = getAppStorage();
    const saved = Array.from({length: storage.length}, (_, index) => storage.key(index)!)
      .filter(key => key.startsWith("research-assistant:writing-prepare:"))
      .map(key => JSON.parse(storage.getItem(key)!));
    expect(saved.some(value => value.intent.requestId === original.requestId && value.intent.text === "不要发送旧场景")).toBe(true);
    expect(originalStart).not.toHaveBeenCalled();
    expect(document.body.textContent).not.toContain("无法准备可编辑副本");
    expect(host.querySelector('[data-action="stop"]')).toBeNull();
  } finally { await act(async () => root.unmount()); host.remove(); }
});
