// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { TextEditorPath } from "./TextEditorPath";

it.each([true, false])("shows a full-path hint only when truncated=%s", async truncated => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockReturnValue(600);
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(truncated ? 200 : 600);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, truncated ? 200 : 600, 24));
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const path = `source://agents/${"nested-directory/".repeat(12)}structured-task.md`;
  try {
    await act(async () => root.render(<TextEditorPath path={path} />));
    const anchor = host.querySelector<HTMLElement>('[tabindex="0"]')!;
    expect(anchor.getAttribute("aria-label")).toBe(path);
    expect(host.querySelector('.text-path-root')?.textContent).toBe("source:");
    expect(host.querySelector('.text-path-tail')?.textContent).toMatch(/structured-task\.md$/);
    await act(async () => { anchor.focus(); await new Promise(resolve => setTimeout(resolve, 400)); });
    if (truncated) expect(document.querySelector('[role="tooltip"]')?.textContent).toBe(path);
    else expect(document.querySelector('[role="tooltip"]')).toBeNull();
  } finally { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); vi.restoreAllMocks(); }
});
