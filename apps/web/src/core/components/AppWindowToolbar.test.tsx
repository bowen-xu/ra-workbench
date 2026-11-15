// @vitest-environment happy-dom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppWindowToolbar } from "./AppWindowToolbar";

const platform = vi.hoisted(() => ({desktop: false}));
vi.mock("../runtime/platformCore", () => ({ isDesktopRuntime: () => platform.desktop }));

describe("AppWindowToolbar", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    platform.desktop = false;
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  it("keeps the same tab host when navigation controls or its visibility change", async () => {
    const hostChanged = vi.fn();
    const props = { canGoBack: false, canGoForward: false, onBack: vi.fn(), onForward: vi.fn(), sidebarExpanded: true, onToggleSidebar: vi.fn(), onTabHostChange: hostChanged };
    await act(async () => root.render(<AppWindowToolbar {...props} tabHostActive />));
    const host = hostChanged.mock.calls[0]![0] as HTMLDivElement;
    expect(host.closest('[aria-label="窗口工具栏"]')).not.toBeNull();
    expect(host.hidden).toBe(false);

    await act(async () => root.render(<AppWindowToolbar {...props} tabHostActive={false} canGoBack />));
    expect(host.hidden).toBe(true);
    await act(async () => root.render(<AppWindowToolbar {...props} tabHostActive sidebarExpanded={false} />));
    expect(host.hidden).toBe(false);
    expect(host.isConnected).toBe(true);
    expect(hostChanged).toHaveBeenCalledTimes(1);
  });

  it("preserves navigation actions and disabled history endpoints with docked tabs", async () => {
    const back = vi.fn();
    const forward = vi.fn();
    const toggle = vi.fn();
    await act(async () => root.render(<AppWindowToolbar canGoBack canGoForward={false} onBack={back} onForward={forward} sidebarExpanded onToggleSidebar={toggle} tabHostActive />));
    const backButton = container.querySelector<HTMLButtonElement>('[aria-label="后退"]')!;
    const forwardButton = container.querySelector<HTMLButtonElement>('[aria-label="前进"]')!;
    const toggleButton = container.querySelector<HTMLButtonElement>('[aria-label="收起主导航"]')!;
    await act(async () => { backButton.click(); forwardButton.click(); toggleButton.click(); });
    expect(back).toHaveBeenCalledOnce();
    expect(forward).not.toHaveBeenCalled();
    expect(toggle).toHaveBeenCalledOnce();
  });

  it("enables native dragging and double-click sizing across toolbar blanks, without marking interactive controls", async () => {
    platform.desktop = true;
    const onBack = vi.fn();
    await act(async () => root.render(<AppWindowToolbar canGoBack canGoForward={false} onBack={onBack} onForward={() => undefined} sidebarExpanded onToggleSidebar={() => undefined} tabHostActive />));
    const toolbar = container.querySelector("header")!;
    expect(toolbar.getAttribute("data-tauri-drag-region")).toBe("deep");
    expect(toolbar.querySelectorAll("[data-tauri-drag-region]")).toHaveLength(0);
    // Tauri skips buttons/roles before reaching a deep ancestor, including portal tabs.
    for (const button of toolbar.querySelectorAll("button")) expect(button.hasAttribute("data-tauri-drag-region")).toBe(false);
    await act(async () => toolbar.querySelector<HTMLButtonElement>('[aria-label="后退"]')!.click());
    expect(onBack).toHaveBeenCalledOnce();
  });

  it("leaves browser and preview toolbars free of native drag regions", async () => {
    await act(async () => root.render(<AppWindowToolbar canGoBack={false} canGoForward={false} onBack={() => undefined} onForward={() => undefined} sidebarExpanded onToggleSidebar={() => undefined} />));
    expect(container.querySelector("[data-tauri-drag-region]")).toBeNull();
  });
});
