// @vitest-environment happy-dom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ContentTrustGroupControls, ContentTrustIndicator } from "./ContentTrustIndicator";

describe("ContentTrustIndicator", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it("shows a shared hint for the reading status dot on hover", async () => {
    vi.useFakeTimers();
    await act(async () => root.render(<ContentTrustIndicator showHint size="small" status="untrusted" subjectLabel="研究问题" statusLabel="AI 生成，未人工审核" />));
    const indicator = container.querySelector('[role="img"]');
    expect(indicator?.getAttribute('aria-label')).toBe("研究问题：AI 生成，未人工审核");
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
    await act(async () => {
      indicator?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
      indicator?.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
      await vi.advanceTimersByTimeAsync(350);
    });
    expect(document.querySelector('[role="tooltip"]:not([hidden])')?.textContent).toContain("研究问题：AI 生成，未人工审核");
  });

  it("always exposes the meaning in text and accessibility metadata", async () => {
    await act(async () => root.render(
      <ContentTrustIndicator showLabel status="untrusted" subjectLabel="关键结果" />,
    ));
    expect(container.textContent).toContain("不可信：AI 生成，尚未人工确认");
    expect(container.querySelector('[role="img"]')?.getAttribute("aria-label"))
      .toBe("关键结果：不可信：AI 生成，尚未人工确认");
  });

  it("aggregates descendants and exposes navigation and batch confirmation", async () => {
    const onConfirmPending = vi.fn();
    const onNavigate = vi.fn();
    await act(async () => root.render(
      <ContentTrustGroupControls
        onConfirmPending={onConfirmPending}
        onNavigate={onNavigate}
        tree={{
          id: "paper",
          label: "文献",
          children: [
            { id: "title", label: "标题", directStatus: "trusted", confirmable: true },
            { id: "doi", label: "DOI", directStatus: "reviewable", confirmable: true },
            { id: "result", label: "关键结果", directStatus: "untrusted", confirmable: true },
          ],
        }}
      />,
    ));
    expect(container.textContent).toContain("红 1");
    expect(container.textContent).toContain("黄 1");
    const buttons = [...container.querySelectorAll("button")];
    await act(async () => buttons.find((button) => button.textContent === "下一个待确认")?.click());
    expect(onNavigate).toHaveBeenCalledWith(expect.objectContaining({ id: "doi" }));
    await act(async () => buttons.find((button) => button.textContent?.startsWith("确认本组"))?.click());
    expect(onConfirmPending).toHaveBeenCalledWith(["doi", "result"]);
  });
});
