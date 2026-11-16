import type {Page} from "@playwright/test";

interface WritingQueryTrace {
  started: Record<string,number>;
  completed: Record<string,number>;
  failed: Record<string,number>;
  pending: number;
}
declare global {interface Window {__writingQueryTrace: WritingQueryTrace}}

/** Install before the production writing workspace mounts. Delegate every
 * operation unchanged; count whole async calls, not just current IDB requests.
 * Names/counts only: no arguments, document content or error payloads.
 */
export async function installWritingQueryTrace(page: Page): Promise<void> {
  await page.evaluate(async () => {
    const {getAppRuntime} = await import("/ra-workbench/apps/web/src/core/runtime/appRuntime.ts");
    const runtime = getAppRuntime(), source = runtime.gateways.getWritingGateway;
    if (!source) throw Error("Synthetic writing gateway is unavailable");
    const trace: WritingQueryTrace = {started:{},completed:{},failed:{},pending:0};
    window.__writingQueryTrace = trace;
    const methods = new Map<PropertyKey,unknown>();
    runtime.gateways.getWritingGateway = new Proxy(source,{
      get(target,key) {
        const value: unknown = Reflect.get(target,key);
        if (typeof value !== "function") return value;
        if (!methods.has(key)) methods.set(key,async (...args: unknown[]) => {
          const name = String(key);
          trace.started[name] = (trace.started[name] ?? 0) + 1;trace.pending++;
          try {
            const result: unknown = await Reflect.apply(value,target,args);
            trace.completed[name] = (trace.completed[name] ?? 0) + 1;
            return result;
          } catch (reason) {trace.failed[name] = (trace.failed[name] ?? 0) + 1;throw reason;}
          finally {trace.pending--;}
        });
        return methods.get(key);
      },
    });
  });
}
export async function readWritingQueryTrace(page: Page): Promise<WritingQueryTrace> {
  return page.evaluate(() => structuredClone(window.__writingQueryTrace));
}
