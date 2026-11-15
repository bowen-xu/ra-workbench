/** Only the small frozen template crosses the UI/native boundary. Font and
 * image bytes are read and encoded by the export job, never the UI thread. */
export type NoteExportResource = {
    readonly token: string;
    readonly kind: "font";
    readonly name: string;
} | {
    readonly token: string;
    readonly kind: "image";
    readonly assetId: string;
};
export interface NoteExportInput {
    readonly template: string;
    readonly resources: readonly NoteExportResource[];
}
export const exportCanvasCss = "html,body{background:#fff!important;color-scheme:light;height:auto!important;min-height:100%;overflow:visible!important}body{margin:0}";
export function checkExportCancelled(signal: AbortSignal): void {
    if (signal.aborted)
        throw new DOMException("导出已停止。", "AbortError");
}
/** Yield between DOM-only batches; worker/desktop adapters own expensive I/O. */
export async function yieldExport(signal: AbortSignal): Promise<void> {
    checkExportCancelled(signal);
    await new Promise<void>(resolve => setTimeout(resolve, 0));
    checkExportCancelled(signal);
}
