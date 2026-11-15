// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppRuntimeProvider } from "../runtime/AppRuntimeProvider";
import type { AppRuntime } from "../runtime/appRuntime";
import { createPreviewControls } from "../runtime/previewControls";
import { PreviewResetControl } from "./PreviewResetControl";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root | undefined;
let container: HTMLDivElement | undefined;
const values = new Map<string, string>();
const storage: Storage = {
  get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
  getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); },
  removeItem: key => { values.delete(key); }, clear: () => values.clear(),
};
afterEach(async () => { await act(async () => root?.unmount()); container?.remove(); });
async function mount(runtime: AppRuntime): Promise<HTMLDivElement> {
  container = document.createElement("div"); document.body.append(container);
  root = createRoot(container);
  await act(async () => root!.render(<AppRuntimeProvider runtime={runtime}><PreviewResetControl /></AppRuntimeProvider>));
  return container;
}

describe("PreviewResetControl", () => {
  it.each(["browser", "desktop"] as const)("is absent from %s mode", async mode => {
    const runtime: AppRuntime = { mode, storage, gateways: {}, dispose() {} };
    const view = await mount(runtime);
    expect(view.querySelector("button")).toBeNull();
  });

  it("uses the runtime's existing reset capability", async () => {
    const reset = vi.fn();
    const runtime: AppRuntime = { mode: "preview", storage, gateways: {},
      preview: createPreviewControls(storage, reset), dispose() {} };
    const view = await mount(runtime);
    await act(async () => view.querySelector<HTMLButtonElement>('[aria-label="重置预览数据"]')!.click());
    expect(reset).toHaveBeenCalledTimes(1);
    expect(view.querySelector('[aria-label="模拟写作错误"]')).toBeNull();
  });
});
