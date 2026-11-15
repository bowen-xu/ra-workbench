// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import {NoteRecoveryPreviewControl} from "./NoteRecoveryPreviewControl";

vi.mock("../runtime/AppRuntimeProvider", () => ({useAppRuntime: () => ({
  noteRecovery: {scenario: "conflict", pageId: "mock-page", releaseFault: vi.fn()},
})}));

it("organizes the actual preview commands into named groups without making headings actions", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<NoteRecoveryPreviewControl/>));
    const trigger = document.querySelector<HTMLButtonElement>('[aria-label="笔记故障预览"]')!;
    await act(async () => trigger.click());
    await vi.waitFor(() => expect(document.querySelector('[role="group"][aria-label="冲突与消解"]')).not.toBeNull());
    const groups = [...document.querySelectorAll('[role="group"]')];
    expect(groups.map(group => group.getAttribute("aria-label"))).toEqual(["冲突与消解", "保存与恢复"]);
    expect(groups.map(group => group.querySelectorAll('[role="menuitem"]').length)).toEqual([4, 6]);
    expect(groups[0]!.querySelector("button")!.disabled).toBe(true);
    expect(groups[0]!.textContent).toContain("消解期间版本变化");
    expect(groups[1]!.textContent).toContain("恢复保全写入失败");
    expect(groups.every(group => group.querySelector(".note-action-menu-group-heading")!.getAttribute("tabindex") === null)).toBe(true);
  } finally {
    await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();
  }
});
