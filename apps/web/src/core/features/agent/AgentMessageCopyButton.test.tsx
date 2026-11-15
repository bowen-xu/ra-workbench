/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentMessageCopyButton } from "./AgentMessageCopyButton";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: vi.fn().mockResolvedValue(undefined) } });
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

it("copies the complete message and exposes a transient success label", async () => {
  const value = ["第一行", "第二行"].join("\n");
  await act(async () => root.render(<AgentMessageCopyButton value={value} />));
  const button = container.querySelector<HTMLButtonElement>("button")!;
  await act(async () => button.click());
  expect(navigator.clipboard.writeText).toHaveBeenCalledWith(value);
  expect(button.getAttribute("aria-label")).toBe("已复制消息");
});
