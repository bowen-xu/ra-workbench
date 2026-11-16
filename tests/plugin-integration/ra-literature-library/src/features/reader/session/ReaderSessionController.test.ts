// @vitest-environment happy-dom

import { beforeEach, describe, expect, it, vi } from "vitest";

import type { PdfViewAdapter, PdfViewAdapterCallbacks } from "../../../../../../../apps/web/src/core/components/pdf-view/PdfViewAdapter";
import { ReaderSessionController } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/session/ReaderSessionController";

function fakeAdapter(): {
  readonly adapter: PdfViewAdapter;
  callbacks(): PdfViewAdapterCallbacks;
  readonly navigate: ReturnType<typeof vi.fn>;
  readonly setActive: ReturnType<typeof vi.fn>;
  readonly trimMemory: ReturnType<typeof vi.fn>;
  readonly suspend: ReturnType<typeof vi.fn>;
} {
  let registered: PdfViewAdapterCallbacks | undefined;
  const navigate = vi.fn(() => true);
  const setActive = vi.fn();
  const trimMemory = vi.fn();
  const suspend = vi.fn();
  const adapter = {
    characterStore: {},
    pageCount: 20,
    open: vi.fn(async (input: { callbacks: PdfViewAdapterCallbacks }) => { registered = input.callbacks; }),
    close: vi.fn(),
    setActive,
    trimMemory,
    suspend,
    resume: vi.fn(),
    navigate,
    focus: vi.fn(),
    changeScale: vi.fn(),
    fitToWidth: vi.fn(),
    updateScaleAt: vi.fn(),
    find: vi.fn(),
    closeFind: vi.fn(),
    extractPageTexts: vi.fn().mockResolvedValue([]),
    renderThumbnail: vi.fn().mockResolvedValue(undefined),
  } as unknown as PdfViewAdapter;
  return {
    adapter,
    callbacks: () => {
      if (registered === undefined) throw new Error("adapter callbacks were not registered");
      return registered;
    },
    navigate,
    setActive,
    trimMemory,
    suspend,
  };
}

describe("ReaderSessionController", () => {
  beforeEach(() => vi.useFakeTimers());

  it("publishes thumbnail page count before main viewer readiness and accepts later labels",async()=>{
    const fake=fakeAdapter();const controller=new ReaderSessionController({adapter:fake.adapter,saveReadingState:vi.fn()});
    await controller.open({bytes:new Uint8Array(),container:document.createElement("div"),viewerElement:document.createElement("div"),active:true});
    fake.callbacks().onDocumentReady?.(20);
    expect(controller.getSnapshot()).toMatchObject({ready:false,pageCount:20});
    fake.callbacks().onPageLabelsChange?.(["i","1"]);
    expect(controller.getSnapshot().pageLabels).toEqual(["i","1"]);
    await controller.close();
  });

  it("passes saved zoom and page offsets through the shared viewer when restoring a session", async () => {
    const fake = fakeAdapter();
    const controller = new ReaderSessionController({ adapter: fake.adapter, saveReadingState: vi.fn() });
    const restored = { schemaVersion: 2 as const, pageIndex: 4, scale: { kind: "numeric" as const, value: 1.25 },
      top: 280, left: 30, scrollMode: "vertical" as const, spreadMode: "none" as const };
    await controller.open({ bytes: new Uint8Array(), container: document.createElement("div"),
      viewerElement: document.createElement("div"), active: true, restoredViewState: restored });
    expect(fake.adapter.open).toHaveBeenLastCalledWith(expect.objectContaining({ initialView: {
      pageIndex: 4, scale: restored.scale, top: 280, left: 30, scrollMode: "vertical", spreadMode: "none",
    }, initialPageIndex: 4 }));
    await controller.close();
  });

  it("lets an explicit source target override saved page offsets without discarding the saved zoom", async () => {
    const fake = fakeAdapter();
    const controller = new ReaderSessionController({ adapter: fake.adapter, saveReadingState: vi.fn() });
    await controller.open({ bytes: new Uint8Array(), container: document.createElement("div"),
      viewerElement: document.createElement("div"), active: true, initialPageIndex: 2,
      restoredViewState: { schemaVersion: 2, pageIndex: 4, scale: { kind: "numeric", value: 1.25 },
        top: 280, left: 30, scrollMode: "vertical", spreadMode: "none" } });
    expect(fake.adapter.open).toHaveBeenLastCalledWith(expect.objectContaining({ initialView: {
      pageIndex: 2, scale: { kind: "numeric", value: 1.25 }, scrollMode: "vertical", spreadMode: "none",
    }, initialPageIndex: 2 }));
    await controller.close();
  });

  it("turns adapter events into a stable session snapshot", async () => {
    const fake = fakeAdapter();
    const controller = new ReaderSessionController({
      adapter: fake.adapter,
      saveReadingState: vi.fn().mockResolvedValue(undefined),
    });
    const listener = vi.fn();
    controller.subscribe(listener);
    await controller.open({
      bytes: new Uint8Array(),
      container: document.createElement("div"),
      viewerElement: document.createElement("div"),
      active: true,
    });
    fake.callbacks().onReady({
      pageCount: 20,
      pageIndex: 2,
      pageLabels: ["i", "ii", "1"],
      scale: 1.25,
      viewState: {
        pageIndex: 2,
        scale: { kind: "page-width" },
        top: 720,
        left: 0,
        scrollMode: "vertical",
        spreadMode: "none",
      },
    });
    fake.callbacks().onPageChange(3);
    fake.callbacks().onFindMatchesChange(2, 7);
    expect(controller.getSnapshot()).toMatchObject({
      ready: true,
      pageCount: 20,
      pageIndex: 3,
      scalePercent: 125,
      findCurrent: 2,
      findTotal: 7,
      lifecycle: "active",
    });
    expect(listener).toHaveBeenCalled();
  });

  it("clamps transient PDF page events before exposing or persisting them", async () => {
    const fake = fakeAdapter();
    const saveReadingState = vi.fn().mockResolvedValue(undefined);
    const controller = new ReaderSessionController({ adapter: fake.adapter, saveReadingState, saveDebounceMs: 1 });
    await controller.open({
      bytes: new Uint8Array(),
      container: document.createElement("div"),
      viewerElement: document.createElement("div"),
      active: true,
    });
    fake.callbacks().onReady({
      pageCount: 3,
      pageIndex: 0,
      pageLabels: [],
      scale: 1,
      viewState: { pageIndex: 0, scale: { kind: "page-fit" }, scrollMode: "vertical", spreadMode: "none" },
    });
    fake.callbacks().onPageChange(450);
    expect(controller.getSnapshot().pageIndex).toBe(2);
    fake.callbacks().onViewAreaChange({
      viewState: { pageIndex: -4, scale: { kind: "page-fit" }, top: 10, left: 0, scrollMode: "vertical", spreadMode: "none" },
    });
    expect(controller.getSnapshot().pageIndex).toBe(0);
    await vi.runAllTimersAsync();
    expect(saveReadingState).toHaveBeenCalledWith(expect.objectContaining({ pageIndex: 0 }));
  });

  it("routes page, label, and source positions through one navigation history", async () => {
    const fake = fakeAdapter();
    const controller = new ReaderSessionController({
      adapter: fake.adapter,
      saveReadingState: vi.fn().mockResolvedValue(undefined),
    });
    await controller.open({
      bytes: new Uint8Array(),
      container: document.createElement("div"),
      viewerElement: document.createElement("div"),
      active: true,
    });
    fake.callbacks().onReady({
      pageCount: 20,
      pageIndex: 0,
      pageLabels: ["i", "ii", "1"],
      scale: 1,
      viewState: { pageIndex: 0, scale: { kind: "page-fit" }, scrollMode: "vertical", spreadMode: "none" },
    });
    expect(controller.navigate({ pageIndex: 4 })).toBe(true);
    vi.advanceTimersByTime(2_100);
    expect(controller.navigate({ pageLabel: "ii" })).toBe(true);
    vi.advanceTimersByTime(2_100);
    expect(controller.navigate({
      position: {
        segments: [{ pageIndex: 8, rects: [], exactText: "source" }],
      },
    })).toBe(true);
    expect(fake.navigate).toHaveBeenNthCalledWith(1, { pageIndex: 4 }, 4);
    expect(fake.navigate).toHaveBeenNthCalledWith(2, { pageLabel: "ii" }, 1);
    expect(fake.navigate).toHaveBeenNthCalledWith(3, {
      position: { segments: [{ pageIndex: 8, rects: [], exactText: "source" }] },
    }, 8);
    expect(controller.getSnapshot().canNavigateBack).toBe(true);
    expect(controller.navigateBack()).toBe(true);
    expect(fake.navigate).toHaveBeenLastCalledWith({ pageLabel: "ii", pageIndex: 1 }, 1);
    expect(controller.navigateForward()).toBe(true);
    expect(fake.navigate).toHaveBeenLastCalledWith({
      position: { segments: [{ pageIndex: 8, rects: [], exactText: "source" }] },
      pageIndex: 8,
    }, 8);
  });

  it("routes named PDF destinations through the same back and forward history", async () => {
    const fake = fakeAdapter();
    const controller = new ReaderSessionController({
      adapter: fake.adapter,
      saveReadingState: vi.fn().mockResolvedValue(undefined),
    });
    await controller.open({
      bytes: new Uint8Array(),
      container: document.createElement("div"),
      viewerElement: document.createElement("div"),
      active: true,
    });
    fake.callbacks().onReady({
      pageCount: 20,
      pageIndex: 2,
      pageLabels: [],
      scale: 1,
      viewState: {
        pageIndex: 2,
        scale: { kind: "numeric", value: 1 },
        top: 700,
        left: 10,
        scrollMode: "vertical",
        spreadMode: "none",
      },
    });
    fake.callbacks().onNavigate?.({ destination: "section-2" });
    expect(controller.getSnapshot().canNavigateBack).toBe(true);
    expect(fake.navigate).toHaveBeenLastCalledWith({ destination: "section-2" }, undefined);
    expect(controller.navigateBack()).toBe(true);
    expect(fake.navigate).toHaveBeenLastCalledWith({
      pageIndex: 2,
      scale: { kind: "numeric", value: 1 },
      top: 700,
      left: 10,
    }, 2);
    expect(controller.navigateForward()).toBe(true);
    expect(fake.navigate).toHaveBeenLastCalledWith({ destination: "section-2" }, undefined);
  });

  it("debounces view persistence, flushes on hide, and keeps a failed state retryable", async () => {
    const fake = fakeAdapter();
    const saveReadingState = vi.fn()
      .mockRejectedValueOnce(new Error("disk busy"))
      .mockResolvedValue(undefined);
    const controller = new ReaderSessionController({
      adapter: fake.adapter,
      saveReadingState,
      saveDebounceMs: 350,
    });
    await controller.open({
      bytes: new Uint8Array(),
      container: document.createElement("div"),
      viewerElement: document.createElement("div"),
      active: true,
    });
    const viewState = {
      schemaVersion: 2 as const,
      pageIndex: 4,
      scale: { kind: "page-width" as const },
      top: 20,
      left: 0,
      scrollMode: "vertical" as const,
      spreadMode: "none" as const,
    };
    fake.callbacks().onViewAreaChange({ viewState });
    vi.advanceTimersByTime(349);
    expect(saveReadingState).not.toHaveBeenCalled();
    controller.setActive(false);
    await vi.runAllTimersAsync();
    expect(fake.setActive).toHaveBeenCalledWith(false);
    expect(saveReadingState).toHaveBeenCalledWith(viewState);
    expect(controller.getSnapshot().status).toContain("disk busy");
    await controller.flushReadingState();
    expect(saveReadingState).toHaveBeenCalledTimes(2);
  });

  it("trims hidden sessions immediately, suspends after 60 seconds, and resumes on activation", async () => {
    const fake = fakeAdapter();
    const controller = new ReaderSessionController({
      adapter: fake.adapter,
      saveReadingState: vi.fn().mockResolvedValue(undefined),
    });
    await controller.open({
      bytes: new Uint8Array(),
      container: document.createElement("div"),
      viewerElement: document.createElement("div"),
      active: true,
    });
    controller.setActive(false);
    expect(fake.setActive).toHaveBeenCalledWith(false);
    expect(controller.getSnapshot().lifecycle).toBe("trimmed");
    vi.advanceTimersByTime(59_999);
    expect(fake.suspend).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(fake.suspend).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().lifecycle).toBe("suspended");

    controller.setActive(true);
    expect(fake.setActive).toHaveBeenCalledWith(true);
    expect(controller.getSnapshot().lifecycle).toBe("active");
  });

  it("exposes a memory-pressure trim without disposing the active session", async () => {
    const fake = fakeAdapter();
    const controller = new ReaderSessionController({
      adapter: fake.adapter,
      saveReadingState: vi.fn().mockResolvedValue(undefined),
    });
    await controller.open({
      bytes: new Uint8Array(),
      container: document.createElement("div"),
      viewerElement: document.createElement("div"),
      active: true,
    });
    controller.trimMemory();
    expect(fake.trimMemory).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().lifecycle).toBe("active");
  });

  it("does not let a stale StrictMode cleanup dispose a newer open intent", async () => {
    const fake = fakeAdapter();
    let finishSave: (() => void) | undefined;
    const controller = new ReaderSessionController({
      adapter: fake.adapter,
      saveReadingState: () => new Promise<void>((resolve) => { finishSave = resolve; }),
    });
    await controller.open({
      bytes: new Uint8Array(),
      container: document.createElement("div"),
      viewerElement: document.createElement("div"),
      active: true,
    });
    fake.callbacks().onViewAreaChange({
      viewState: {
        pageIndex: 1,
        scale: { kind: "page-fit" },
        scrollMode: "vertical",
        spreadMode: "none",
      },
    });
    const staleClose = controller.close();
    await Promise.resolve();
    controller.prepareOpen(true, 4);
    finishSave?.();
    await staleClose;
    expect(controller.getSnapshot()).toMatchObject({ lifecycle: "active", pageIndex: 4 });
    expect(fake.adapter.close).not.toHaveBeenCalled();
  });
});
