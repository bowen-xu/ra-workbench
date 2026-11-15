import { buildNoteHtmlBlob } from "./buildNoteHtmlBlob";
import type { NoteExportInput } from "./noteExportInput";
export interface ExportWorkerRequest {
    readonly input: NoteExportInput;
    readonly fontBaseUrl: string;
    readonly images: Readonly<Record<string, ArrayBuffer>>;
}
export type ExportWorkerReply = {
    readonly kind: "done";
    readonly blob: Blob;
} | {
    readonly kind: "error";
    readonly message: string;
};
const worker = self as unknown as {
    onmessage: ((event: MessageEvent<ExportWorkerRequest>) => void) | null;
    postMessage(message: ExportWorkerReply): void;
};
worker.onmessage = event => {
    const request = event.data;
    void buildNoteHtmlBlob(request.input, async (resource) => {
        if (resource.kind === "image") {
            const bytes = request.images[resource.assetId];
            if (!bytes)
                throw new Error("NOTE_EXPORT_IMAGE_UNAVAILABLE：图片数据缺失。");
            return new Uint8Array(bytes);
        }
        if (!/^(?:ResearchAssistantSans-(?:Regular|Bold)|KaTeX_[A-Za-z]+[1-4]?-(?:Regular|Bold|Italic|BoldItalic))\.ttf$/u.test(resource.name)) {
            throw new Error("NOTE_EXPORT_FONT_INVALID：字体名称无效。");
        }
        const response = await fetch(new URL(resource.name, request.fontBaseUrl));
        if (!response.ok)
            throw new Error(`NOTE_EXPORT_FONT_UNAVAILABLE：${resource.name}; HTTP ${response.status}`);
        return new Uint8Array(await response.arrayBuffer());
    }).then(blob => worker.postMessage({ kind: "done", blob }), cause => worker.postMessage({ kind: "error", message: cause instanceof Error ? cause.message : String(cause) }));
};
