/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AgentCanvasDemo } from "./AgentCanvasDemo";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("uses live pacing controls and retains them across scenario switches and resets", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const input = (label: string) => host.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;
  async function set(label: string, value: string) {
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input(label), value);
      input(label).dispatchEvent(new Event("input", { bubbles: true }));
    });
  }
  async function click(label: string) {
    await act(async () => [...host.querySelectorAll('button')].find(button => button.textContent === label)!.click());
  }
  async function selectScenario(value: string) {
    await act(async () => {
      const select = host.querySelector<HTMLSelectElement>('[aria-label="模拟消息场景"]')!;
      select.value = value; select.dispatchEvent(new Event("change", { bubbles: true }));
    });
  }
  async function ready() {
    await vi.waitFor(async () => {
      await act(async () => Promise.resolve());
      expect(host.querySelector('[data-action="stop"]')).not.toBeNull();
      expect(host.querySelector<HTMLTextAreaElement>('textarea')?.disabled).toBe(false);
    }, { timeout: 4000 });
  }
  try {
    await act(async () => root.render(<AgentCanvasDemo />));
    await vi.waitFor(async () => {
      await act(async () => new Promise(resolve => setTimeout(resolve, 10)));
      expect(host.querySelector('[data-action="stop"]')).not.toBeNull();
    }, { timeout: 4000 });
    expect(input("每次输出字数").value).toBe("3");
    expect(input("输出间隔").value).toBe("0.3");
    expect(input("渐显时长").value).toBe("0.3");
    expect(input("模拟压缩时长").value).toBe("1");
    await set("每次输出字数", "2"); await set("输出间隔", "0.5"); await set("渐显时长", "1.2");
    await set("模拟压缩时长", "5");
    await selectScenario("single");
    await vi.waitFor(async () => {
      await act(async () => new Promise(resolve => setTimeout(resolve, 10)));
      expect(host.querySelector('[data-agent-activity="thinking"]')).not.toBeNull();
      expect(host.querySelector<HTMLTextAreaElement>('textarea')?.disabled).toBe(false);
    }, { timeout: 4000 });
    vi.useFakeTimers();
    await act(async () => {
      const duration = host.querySelector<HTMLSelectElement>('[aria-label="模拟运行时长"]')!;
      duration.value = "0.05"; duration.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await act(async () => vi.advanceTimersByTimeAsync(3000));
    expect(host.querySelector('.writing-agent-message.assistant .writing-agent-markdown')?.textContent).toBe("这是");
    await act(async () => vi.advanceTimersByTimeAsync(499));
    expect(host.querySelector('.writing-agent-message.assistant .writing-agent-markdown')?.textContent).toBe("这是");
    await act(async () => vi.advanceTimersByTimeAsync(1));
    expect(host.querySelector('.writing-agent-message.assistant .writing-agent-markdown')?.textContent).toBe("这是模拟");
    await set("每次输出字数", "5"); await set("输出间隔", "0.1");
    await act(async () => vi.advanceTimersByTimeAsync(100));
    expect(host.querySelector('.writing-agent-message.assistant .writing-agent-markdown')?.textContent).toBe("这是模拟的单条文本");
    await set("渐显时长", "0");
    expect([...host.querySelectorAll<HTMLElement>('.stream-reveal-glyph')].every(glyph =>
      glyph.style.getPropertyValue("--stream-reveal-progress") === "1")).toBe(true);
    await click("重置假数据");
    await ready();
    expect(input("每次输出字数").value).toBe("5");
    expect(input("输出间隔").value).toBe("0.1");
    expect(input("渐显时长").value).toBe("0");
    expect(input("模拟压缩时长").value).toBe("5");
    await selectScenario("markdown");
    await ready();
    expect(input("每次输出字数").value).toBe("5");
    expect(input("输出间隔").value).toBe("0.1");
    expect(input("渐显时长").value).toBe("0");
    expect(input("模拟压缩时长").value).toBe("5");
  } finally { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); }
}, 10000);
