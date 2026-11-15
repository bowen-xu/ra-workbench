// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { buildBrowserNoteExport } from "./browserNoteExport";
afterEach(() => vi.unstubAllGlobals());
class ExportWorker {
    static current: ExportWorker;
    onmessage?: (event: MessageEvent) => void;
    onerror?: () => void;
    postMessage = vi.fn();
    terminate = vi.fn();
    constructor() { ExportWorker.current = this; }
}
it("stops the actual worker and transfers copies without detaching gateway resources", async () => {
    vi.stubGlobal("Worker", ExportWorker);
    const bytes = new Uint8Array([1, 2, 3]);
    const controller = new AbortController();
    const pending = buildBrowserNoteExport({ template: "template", resources: [{ kind: "image", token: "image", assetId: "asset" }] }, async () => bytes, controller.signal);
    await vi.waitFor(() => expect(ExportWorker.current.postMessage).toHaveBeenCalledOnce());
    const [request, transfer] = ExportWorker.current.postMessage.mock.calls[0]!;
    expect(request.images.asset).not.toBe(bytes.buffer);
    expect(transfer).toEqual([request.images.asset]);
    controller.abort();
    await expect(pending).rejects.toMatchObject({ name: "AbortError" });
    expect(ExportWorker.current.terminate).toHaveBeenCalledOnce();
    expect([...bytes]).toEqual([1, 2, 3]);
});
it("can stop while a gateway read is pending, without starting a worker later", async () => {
    const constructor = vi.fn();
    vi.stubGlobal("Worker", constructor);
    const controller = new AbortController();
    let complete!: (bytes: Uint8Array) => void;
    const pending = buildBrowserNoteExport({ template: "template", resources: [{ kind: "image", token: "image", assetId: "asset" }] }, () => new Promise(resolve => { complete = resolve; }), controller.signal);
    controller.abort();
    await expect(pending).rejects.toMatchObject({ name: "AbortError" });
    complete(new Uint8Array([1]));
    await Promise.resolve();
    expect(constructor).not.toHaveBeenCalled();
});
