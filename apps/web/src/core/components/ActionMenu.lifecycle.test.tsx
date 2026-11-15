// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { ActionMenu, ActionMenuItem } from "./ActionMenu";

it("does not render closed command content and opens with the latest action", async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container), renderContent = vi.fn(), first = vi.fn(), latest = vi.fn();
  function Content({ action }: { readonly action: () => void }) {
    renderContent();
    return <ActionMenuItem onClick={action}>执行最新操作</ActionMenuItem>;
  }
  const menu = (open: boolean, action: () => void) => <ActionMenu label="更多操作" trigger="菜单" className="test-menu" open={open}>
    <Content action={action} />
  </ActionMenu>;
  try {
    await act(async () => root.render(menu(false, first)));
    for (let index = 0; index < 20; index++) await act(async () => root.render(menu(false, latest)));
    expect(renderContent).not.toHaveBeenCalled();
    await act(async () => root.render(menu(true, latest)));
    const action = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(node => node.textContent === "执行最新操作");
    expect(action).toBeDefined();
    await act(async () => action!.click());
    expect(latest).toHaveBeenCalledOnce(); expect(first).not.toHaveBeenCalled();
    await act(async () => root.render(menu(false, latest)));
    expect(document.querySelector('[role="menuitem"]')).toBeNull();
  } finally {
    await act(async () => root.unmount()); container.remove();
  }
});

it("retains stateful picker content across closing when its host requests it", async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container), initialize = vi.fn();
  function Picker() {
    const [value, setValue] = useState(() => { initialize(); return 0; });
    return <ActionMenuItem hideOnClick={false} onClick={() => setValue(previous => previous + 1)}>选择 {value}</ActionMenuItem>;
  }
  const menu = (open: boolean) => <ActionMenu keepMounted label="资源选择" trigger="选择" className="test-menu" open={open}><Picker /></ActionMenu>;
  try {
    await act(async () => root.render(menu(true)));
    await act(async () => (document.querySelector('[role="menuitem"]') as HTMLButtonElement).click());
    await act(async () => root.render(menu(false)));
    await act(async () => root.render(menu(true)));
    expect(document.querySelector('[role="menuitem"]')?.textContent).toBe("选择 1");
    expect(initialize).toHaveBeenCalledOnce();
  } finally { await act(async () => root.unmount()); container.remove(); }
});
