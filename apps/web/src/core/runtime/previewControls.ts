import type { PreviewControls } from "./appRuntime";

export function createPreviewControls(storage: Storage, reset: () => void | Promise<void>): PreviewControls {
  const saved = JSON.parse(storage.getItem("controls") ?? "null") as { delayMs: number; nextFailure: string | null } | null;
  let delayMs = saved?.delayMs ?? 100;
  let nextFailure = saved?.nextFailure ?? null;
  let disposed = false;
  return {
    get delayMs() { return delayMs; }, get nextFailure() { return nextFailure; }, reset,
    dispose: () => { disposed = true; },
    configure(values) {
      if (!Number.isFinite(values.delayMs) || values.delayMs < 0 || values.delayMs > 10000) throw new Error("模拟延迟必须在 0–10000 毫秒之间");
      delayMs = values.delayMs; nextFailure = values.nextFailure;
      storage.setItem("controls", JSON.stringify({ delayMs, nextFailure }));
    },
    async beforeOperation(name) {
      if (disposed) throw new Error("模拟运行环境已关闭");
      const fail = nextFailure === "any" || nextFailure === name;
      if (fail) { nextFailure = null; storage.setItem("controls", JSON.stringify({ delayMs, nextFailure })); }
      if (delayMs) await new Promise<void>(resolve => setTimeout(resolve, delayMs));
      if (disposed) throw new Error("模拟运行环境已关闭");
      if (fail) throw new Error(`模拟操作失败：${name}。可以通过现有操作入口重试。`);
    },
  };
}
