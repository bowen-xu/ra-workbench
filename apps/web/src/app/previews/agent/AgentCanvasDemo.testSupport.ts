import { act } from "react";
import { expect, vi } from "vitest";

/** Populate the production queue through the same composer action as a user. */
export async function enqueueCanvasMessage(host: HTMLElement, text: string): Promise<void> {
  const composer = () => host.querySelector<HTMLTextAreaElement>('[aria-label="给 AI 写作助手发送消息"]');
  const send = () => host.querySelector<HTMLButtonElement>('[data-action="send"]');
  await vi.waitFor(async () => {
    await act(async () => Promise.resolve());
    expect(composer()?.disabled).toBe(false);
  }, { timeout: 4000 });
  await act(async () => {
    const textarea = composer()!;
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, text);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await vi.waitFor(() => expect(send()?.disabled).toBe(false), { timeout: 4000 });
  await act(async () => send()!.click());
  await vi.waitFor(async () => {
    await act(async () => Promise.resolve());
    expect([...host.querySelectorAll('.agent-input-queue-body li p')].some(row => row.textContent === text)).toBe(true);
  }, { timeout: 4000 });
}
