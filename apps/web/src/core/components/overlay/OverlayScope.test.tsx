// @vitest-environment happy-dom
import { act, StrictMode, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import { Dialog } from "./OverlayDialog";
import { ActionMenu, ActionMenuItem } from "../ActionMenu";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

function ReopenableDialog() {
  const [open, setOpen] = useState(true);
  return <><button onClick={() => setOpen(true)}>打开</button>
    <Dialog open={open} onClose={() => setOpen(false)} aria-label="测试弹窗">
      <button onClick={() => setOpen(false)}>关闭</button>
      <ActionMenu label="操作" className="menu" trigger="菜单" open>
        <ActionMenuItem>执行</ActionMenuItem>
      </ActionMenu>
    </Dialog></>;
}

it("owns popup portals outside the modal content and cleans them across StrictMode reopen/unmount", async () => {
  await act(async () => root.render(<StrictMode><ReopenableDialog /></StrictMode>));
  for (let cycle = 0; cycle < 3; cycle++) {
    const dialog = document.querySelector('[aria-label="测试弹窗"]')!;
    const menu = document.querySelector('[role="menu"]')!;
    expect(menu).not.toBeNull();
    const scope = dialog.closest("[data-overlay-scope]");
    expect(scope?.parentElement).toBe(document.body);
    expect(menu.closest("[data-overlay-scope]")).toBe(scope);
    expect(dialog.contains(menu)).toBe(false);
    expect(document.querySelectorAll("[data-overlay-scope]")).toHaveLength(1);
    await act(async () => Array.from(dialog.querySelectorAll("button")).find(button => button.textContent === "关闭")!.click());
    expect(document.querySelectorAll("[data-overlay-scope]")).toHaveLength(0);
    expect(document.querySelectorAll("[data-overlay-portal]")).toHaveLength(0);
    await act(async () => host.querySelector("button")!.click());
  }
  await act(async () => root.render(null));
  expect(document.querySelectorAll("[data-overlay-scope], [data-overlay-portal]")).toHaveLength(0);
});
