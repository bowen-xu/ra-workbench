// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emptyLiteratureDraft, type LiteratureItem } from "@research-assistant/core/literature";

import { useBrowserCaptureRefresh } from "../../../../../../plugins/ra-literature-library/src/components/useBrowserCaptureRefresh";

const native = vi.hoisted(() => ({ desktop: true, subscribe: vi.fn() }));
vi.mock("../../../../../apps/web/src/core/runtime/platformCore", () => ({ isDesktopRuntime: () => native.desktop }));
vi.mock("../../../../../apps/web/src/core/services/browserConnectorGateway", () => ({ listenBrowserCaptures: native.subscribe }));

function item(id: string): LiteratureItem {
  return { ...emptyLiteratureDraft(), id, title: id, attachments: [], projectIds: [], metadataSources: [],
    createdAt: "2026-10-16T00:00:00Z", updatedAt: "2026-10-16T00:00:00Z", readingStatus: "unread" };
}
function deferred<T>() {
  let resolve: (value: T) => void = () => { throw new Error("Promise not initialized"); };
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

describe("browser capture library refresh", () => {
  let host: HTMLDivElement, root: Root;
  let capture: (() => void) | undefined;
  const gateway = { list: vi.fn(), listTrash: vi.fn(), listDuplicates: vi.fn() };
  const onError = vi.fn(), stop = vi.fn(), onDuplicates = vi.fn();
  function Harness({ query = "", showTrash = false }: { query?: string; showTrash?: boolean }) {
    const [items, setItems] = useState<readonly LiteratureItem[]>([item("selected")]);
    const [selection] = useState("selected"), [draft] = useState("draft remains");
    useBrowserCaptureRefresh({ gateway, query, showTrash, onItems: setItems, onDuplicates, onError });
    return <div><output>{items.map(value => value.id).join(",")}</output><span>{selection}</span><input value={draft} readOnly /></div>;
  }
  beforeEach(() => {
    vi.clearAllMocks(); native.desktop = true; capture = undefined;
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    native.subscribe.mockImplementation(async (callback: () => void) => { capture = callback; return stop; });
    gateway.list.mockResolvedValue([item("selected"), item("captured")]);
    gateway.listTrash.mockResolvedValue([item("trash")]);
    gateway.listDuplicates.mockResolvedValue([]);
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

  it("refreshes real list and duplicate data without changing selection or local drafts", async () => {
    await act(async () => root.render(<Harness query="paper" />));
    expect(gateway.list).not.toHaveBeenCalled();
    await act(async () => capture?.());
    expect(gateway.list).toHaveBeenCalledWith("paper");
    expect(host.querySelector("output")?.textContent).toBe("selected,captured");
    expect(host.querySelector("span")?.textContent).toBe("selected");
    expect(host.querySelector("input")?.value).toBe("draft remains");
    expect(onDuplicates).toHaveBeenCalledWith([]);
    expect(onError).not.toHaveBeenCalled();
  });

  it("coalesces bursts and refuses a result for a previous search or trash view", async () => {
    const first = deferred<readonly LiteratureItem[]>();
    gateway.list.mockReturnValueOnce(first.promise);
    await act(async () => root.render(<Harness query="old" />));
    await act(async () => { capture?.(); capture?.(); });
    expect(gateway.list).toHaveBeenCalledTimes(1);
    await act(async () => root.render(<Harness query="new" showTrash />));
    await act(async () => first.resolve([item("stale-result")]));
    expect(gateway.listTrash).toHaveBeenCalledExactlyOnceWith("new");
    expect(host.querySelector("output")?.textContent).toBe("trash");
    expect(onDuplicates).toHaveBeenCalledTimes(1);
  });

  it("disposes a late subscription and never subscribes in browser previews", async () => {
    const subscription = deferred<() => void>();
    native.subscribe.mockReturnValueOnce(subscription.promise);
    await act(async () => root.render(<Harness />));
    await act(async () => root.render(null));
    await act(async () => subscription.resolve(stop));
    expect(stop).toHaveBeenCalledOnce();
    native.desktop = false;
    await act(async () => root.render(<Harness />));
    expect(native.subscribe).toHaveBeenCalledTimes(1);
  });

  it("reports refresh failure without replacing the currently displayed data", async () => {
    gateway.list.mockRejectedValueOnce(new Error("native diagnostic"));
    await act(async () => root.render(<Harness />));
    await act(async () => capture?.());
    expect(host.querySelector("output")?.textContent).toBe("selected");
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("刷新失败"));
  });
});
