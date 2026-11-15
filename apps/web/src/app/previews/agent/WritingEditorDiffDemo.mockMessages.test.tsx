/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { WritingEditorDiffDemo } from "./WritingEditorDiffDemo";

vi.mock("../../../core/features/agent/presentation/WritingTextDiff", () => ({
  WritingTextDiff: () => <div data-testid="review-diff">review diff</div>,
}));

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  document.body.innerHTML = "";
});

it("renders durable tool activity through the production history controller", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<WritingEditorDiffDemo />));
    await vi.waitFor(async () => {
      await act(async () => { await Promise.resolve(); });
      expect(host.querySelector('[data-agent-activity-record] [data-tool-status]')).not.toBeNull();
    }, { timeout: 4000 });
    const messages = [...host.querySelectorAll('[data-agent-message-id]')];
    expect(messages.length).toBeGreaterThan(0);
    expect(messages.every(message => message.textContent?.trim())).toBe(true);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
