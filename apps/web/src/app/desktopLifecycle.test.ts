import { beforeEach, describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  invoke: vi.fn(async () => undefined),
  isTauri: vi.fn(() => true),
  listen: vi.fn(),
  getCurrentWindow: vi.fn(),
  hide: vi.fn(async () => undefined),
  isVisible: vi.fn(async () => false),
  close: vi.fn(async () => undefined),
  onCloseRequested: vi.fn(),
  quitHandler: undefined as (() => Promise<void>) | undefined,
  closeHandler: undefined as ((event: { preventDefault(): void }) => Promise<void>) | undefined,
  keydownHandler: undefined as ((event: KeyboardEvent) => void) | undefined,
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: tauriMocks.invoke,
  isTauri: tauriMocks.isTauri,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: tauriMocks.listen,
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: tauriMocks.getCurrentWindow,
}));

import { getReaderCloseCoordinator } from "../core/workspace/DocumentCloseCoordinator";
import { desktopCloseIntent, installDesktopLifecycle } from "./desktopLifecycle";
import {selectAppStorage} from "../core/runtime/appStorage";
import {registerStoragePersistence} from "../core/runtime/storagePersistence";

beforeEach(() => {
  selectAppStorage({length:0,key:()=>null,getItem:()=>null,setItem:()=>{},removeItem:()=>{},clear:()=>{}});
  vi.clearAllMocks();
  tauriMocks.quitHandler = undefined;
  tauriMocks.closeHandler = undefined;
  tauriMocks.keydownHandler = undefined;
  tauriMocks.listen.mockImplementation(async (_event: string, handler: () => Promise<void>) => {
    tauriMocks.quitHandler = handler;
    return vi.fn();
  });
  tauriMocks.onCloseRequested.mockImplementation(async (
    handler: (event: { preventDefault(): void }) => Promise<void>,
  ) => {
    tauriMocks.closeHandler = handler;
    return vi.fn();
  });
  tauriMocks.getCurrentWindow.mockReturnValue({
    hide: tauriMocks.hide,
    isVisible: tauriMocks.isVisible,
    close: tauriMocks.close,
    onCloseRequested: tauriMocks.onCloseRequested,
  });
  vi.stubGlobal("window", {
    addEventListener: vi.fn((event: string, handler: (event: KeyboardEvent) => void) => {
      if (event === "keydown") tauriMocks.keydownHandler = handler;
    }),
    removeEventListener: vi.fn(),
  });
});

describe("desktop reader lifecycle policy", () => {
  it("waits for workspace preference persistence and refuses quit on storage failure",async()=>{
    const flush=vi.fn().mockRejectedValueOnce(new Error("WORKSPACE_STORAGE_FAILED")).mockResolvedValue(undefined);
    const storage:Storage={length:0,key:()=>null,getItem:()=>null,setItem:()=>{},removeItem:()=>{},clear:()=>{}};
    registerStoragePersistence(storage,{backend:"IndexedDB",pending:true,error:undefined,resetting:false,flush,retry:async()=>{},reset:async()=>{},subscribe:()=>()=>{},close:()=>{}});
    selectAppStorage(storage);
    const onError=vi.fn(),stop=await installDesktopLifecycle({onError});
    try {
      await tauriMocks.quitHandler?.();
      expect(tauriMocks.invoke).not.toHaveBeenCalled();expect(onError).toHaveBeenCalledWith(expect.stringContaining("WORKSPACE_STORAGE_FAILED"));
      await tauriMocks.quitHandler?.();expect(flush).toHaveBeenCalledTimes(2);expect(tauriMocks.invoke).toHaveBeenCalledWith("confirm_app_quit");
    } finally {stop();}
  });
  it("maps Cmd+W to the active PDF tab before the native window", async () => {
    expect(desktopCloseIntent(true)).toBe("reader-tab");
    expect(desktopCloseIntent(false)).toBe("window");
    const commit = vi.fn();
    const unregister = getReaderCloseCoordinator().register("desktop-active", {
      isActive: () => true,
      prepare: vi.fn().mockResolvedValue(true),
      commit,
    });
    const uninstall = await installDesktopLifecycle();
    try {
      let prevented = false;
      tauriMocks.keydownHandler?.({
        key: "w",
        metaKey: true,
        ctrlKey: false,
        altKey: false,
        defaultPrevented: false,
        preventDefault: () => { prevented = true; },
      } as KeyboardEvent);
      await vi.waitFor(() => expect(commit).toHaveBeenCalledOnce());
      expect(prevented).toBe(true);
      expect(tauriMocks.close).not.toHaveBeenCalled();
    } finally {
      uninstall();
      unregister();
    }
  });

  it("prepares and hides the window for Cmd+W when no reader tab is active", async () => {
    const uninstall = await installDesktopLifecycle();
    try {
      tauriMocks.keydownHandler?.({
        key: "w",
        metaKey: true,
        ctrlKey: false,
        altKey: false,
        defaultPrevented: false,
        preventDefault: vi.fn(),
      } as unknown as KeyboardEvent);
      await vi.waitFor(() => expect(tauriMocks.hide).toHaveBeenCalledOnce());
      expect(tauriMocks.isVisible).toHaveBeenCalledOnce();
      expect(tauriMocks.close).not.toHaveBeenCalled();
      expect(tauriMocks.invoke).not.toHaveBeenCalled();
    } finally {
      uninstall();
    }
  });

  it("prepares sessions before hiding a requested window", async () => {
    const prepare = vi.fn().mockResolvedValue(true);
    const unregister = getReaderCloseCoordinator().register("desktop-window", {
      isActive: () => false,
      prepare,
      commit: vi.fn(),
    });
    const uninstall = await installDesktopLifecycle();
    try {
      const preventDefault = vi.fn();
      await tauriMocks.closeHandler?.({ preventDefault });
      expect(preventDefault).toHaveBeenCalledOnce();
      expect(prepare).toHaveBeenCalledOnce();
      expect(tauriMocks.hide).toHaveBeenCalledOnce();
      expect(tauriMocks.isVisible).toHaveBeenCalledOnce();
      expect(prepare.mock.invocationCallOrder[0]).toBeLessThan(tauriMocks.hide.mock.invocationCallOrder[0] ?? 0);
      expect(tauriMocks.hide.mock.invocationCallOrder[0]).toBeLessThan(tauriMocks.isVisible.mock.invocationCallOrder[0] ?? 0);
    } finally {
      uninstall();
      unregister();
    }
  });

  it("prepares sessions before confirming a native app quit", async () => {
    const prepare = vi.fn().mockResolvedValue(true);
    const unregister = getReaderCloseCoordinator().register("desktop-quit", {
      isActive: () => false,
      prepare,
      commit: vi.fn(),
    });
    const uninstall = await installDesktopLifecycle();
    try {
      await tauriMocks.quitHandler?.();
      expect(prepare).toHaveBeenCalledOnce();
      expect(tauriMocks.invoke).toHaveBeenCalledWith("confirm_app_quit");
      expect(prepare.mock.invocationCallOrder[0]).toBeLessThan(tauriMocks.invoke.mock.invocationCallOrder[0] ?? 0);
    } finally {
      uninstall();
      unregister();
    }
  });

  it("keeps the window open when durable recovery preparation fails", async () => {
    const unregister = getReaderCloseCoordinator().register("desktop-recovery-failure", {
      isActive: () => false,
      prepare: vi.fn().mockResolvedValue(false),
      commit: vi.fn(),
    });
    const uninstall = await installDesktopLifecycle();
    try {
      await tauriMocks.closeHandler?.({ preventDefault: vi.fn() });
      await tauriMocks.quitHandler?.();
      expect(tauriMocks.hide).not.toHaveBeenCalled();
      expect(tauriMocks.invoke).not.toHaveBeenCalled();
    } finally {
      uninstall();
      unregister();
    }
  });

  it("reports native Agent shutdown failure and permits another quit attempt", async () => {
    const onError = vi.fn();
    tauriMocks.invoke.mockRejectedValueOnce(new Error("Agent 会话尚未完成保存，请稍后重试退出。"));
    const uninstall = await installDesktopLifecycle({ onError });
    try {
      await tauriMocks.quitHandler?.();
      expect(onError).toHaveBeenCalledWith("退出应用失败：Agent 会话尚未完成保存，请稍后重试退出。");
      expect(tauriMocks.hide).not.toHaveBeenCalled();
      await tauriMocks.quitHandler?.();
      expect(tauriMocks.invoke).toHaveBeenCalledTimes(2);
    } finally {
      uninstall();
    }
  });

  it("reports a failed hide and releases the lock so closing can be retried", async () => {
    const onError = vi.fn();
    tauriMocks.hide.mockRejectedValueOnce(new Error("window.hide not allowed"));
    const uninstall = await installDesktopLifecycle({ onError });
    try {
      await tauriMocks.closeHandler?.({ preventDefault: vi.fn() });
      expect(onError).toHaveBeenCalledWith("关闭窗口失败：window.hide not allowed");

      await tauriMocks.closeHandler?.({ preventDefault: vi.fn() });
      expect(tauriMocks.hide).toHaveBeenCalledTimes(2);
      expect(tauriMocks.isVisible).toHaveBeenCalledOnce();
    } finally {
      uninstall();
    }
  });

  it("reports when the native window remains visible after hide", async () => {
    const onError = vi.fn();
    tauriMocks.isVisible.mockResolvedValueOnce(true);
    const uninstall = await installDesktopLifecycle({ onError });
    try {
      await tauriMocks.closeHandler?.({ preventDefault: vi.fn() });
      expect(onError).toHaveBeenCalledWith("关闭窗口失败：窗口仍然可见，请重试");
    } finally {
      uninstall();
    }
  });
});
