import { checkExportCancelled, type NoteExportInput } from "./noteExportInput";
import type { ExportWorkerReply, ExportWorkerRequest } from "./noteExport.worker";
function abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
    checkExportCancelled(signal);
    return new Promise((resolve, reject) => {
        const abort = () => { cleanup(); reject(new DOMException("导出已停止。", "AbortError")); };
        const cleanup = () => signal.removeEventListener("abort", abort);
        signal.addEventListener("abort", abort, { once: true });
        promise.then(value => { cleanup(); if (signal.aborted)
            abort();
        else
            resolve(value); }, cause => { cleanup(); reject(cause); });
    });
}
export async function buildBrowserNoteExport(input: NoteExportInput, readAsset: (id: string) => Promise<Uint8Array>, signal: AbortSignal): Promise<Blob> {
    checkExportCancelled(signal);
    if (typeof Worker === "undefined")
        throw new Error("NOTE_EXPORT_WORKER_UNAVAILABLE：浏览器不支持后台导出。");
    const images: Record<string, ArrayBuffer> = {};
    for (const resource of input.resources) {
        if (resource.kind !== "image" || images[resource.assetId])
            continue;
        const bytes = await abortable(readAsset(resource.assetId), signal);
        checkExportCancelled(signal);
        // Never detach a gateway/cache-owned buffer when transferring ownership.
        images[resource.assetId] = bytes.slice().buffer as ArrayBuffer;
    }
    checkExportCancelled(signal);
    const worker = new Worker(new URL("./noteExport.worker.ts", import.meta.url), { type: "module" });
    return new Promise((resolve, reject) => {
        const cleanup = () => { signal.removeEventListener("abort", abort); worker.terminate(); };
        const abort = () => { cleanup(); reject(new DOMException("导出已停止。", "AbortError")); };
        signal.addEventListener("abort", abort, { once: true });
        worker.onerror = () => { cleanup(); reject(new Error("NOTE_EXPORT_WORKER_FAILED：后台导出线程无法运行。")); };
        worker.onmessage = (event: MessageEvent<ExportWorkerReply>) => {
            const result = event.data;
            cleanup();
            if (signal.aborted) {
                reject(new DOMException("导出已停止。", "AbortError"));
                return;
            }
            if (result.kind === "done" && result.blob instanceof Blob)
                resolve(result.blob);
            else
                reject(new Error(result.kind === "error" ? result.message : "NOTE_EXPORT_WORKER_PROTOCOL：后台导出回执无效。"));
        };
        const request: ExportWorkerRequest = { input, fontBaseUrl: new URL("resources/note-pdf-fonts/", document.baseURI).href, images };
        try {
            worker.postMessage(request, Object.values(images));
        }
        catch (cause) {
            cleanup();
            reject(new Error("NOTE_EXPORT_WORKER_START：无法传递后台导出输入。", { cause }));
        }
    });
}
