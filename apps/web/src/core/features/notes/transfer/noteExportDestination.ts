import { getAppRuntime } from "../../../runtime/appRuntime";
import { save } from "../../../runtime/platformDialog";
import { downloadDir, join } from "../../../runtime/platformPath";
import { checkExportCancelled } from "./noteExportInput";
export type NoteExportFormat = "pdf" | "html";
export type NoteExportDestination = {
    readonly kind: "desktop";
    readonly path: string;
} | {
    readonly kind: "browser";
    write(blob: Blob, signal: AbortSignal): Promise<"saved" | "download-started">;
};
export function noteExportFileName(title: string, format: NoteExportFormat): string {
    const stem = title.replace(/[\u0000-\u001f\u007f\\/:*?"<>|]/gu, "-").trim().replace(/^[.]+|[. ]+$/gu, "") || "无标题";
    let result = "";
    for (const character of stem) {
        if (new TextEncoder().encode(result + character).length > 200)
            break;
        result += character;
    }
    if (/^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/iu.test(result))
        result = `_${result}`;
    return `${result}.${format}`;
}
type FileHandle = {
    createWritable(): Promise<{
        write(blob: Blob): Promise<void>;
        close(): Promise<void>;
        abort(): Promise<void>;
    }>;
};
type PickerWindow = Window & {
    showSaveFilePicker?: (options: {
        suggestedName: string;
        startIn: "downloads";
        types: {
            description: string;
            accept: Record<string, string[]>;
        }[];
    }) => Promise<FileHandle>;
};
/** Native jobs own publication. Browser writers take a worker-owned Blob,
 * avoiding UI-thread byte conversion; choosing first also preserves activation. */
export async function chooseNoteExportDestination(name: string, format: NoteExportFormat): Promise<NoteExportDestination | undefined> {
    const label = format.toUpperCase();
    if (getAppRuntime().mode === "desktop") {
        const path = await save({ title: `导出笔记 ${label}`, defaultPath: await join(await downloadDir(), name), filters: [{ name: label, extensions: [format] }] });
        return path ? { kind: "desktop", path } : undefined;
    }
    if (format === "pdf")
        throw new Error("NOTE_PDF_DESKTOP_ONLY：PDF 导出仅在桌面应用可用。");
    const picker = (window as PickerWindow).showSaveFilePicker;
    if (picker) {
        let handle: FileHandle;
        try {
            handle = await picker.call(window, { suggestedName: name, startIn: "downloads", types: [{ description: "HTML", accept: { "text/html": [".html"] } }] });
        }
        catch (cause) {
            if (cause instanceof DOMException && cause.name === "AbortError")
                return undefined;
            throw cause;
        }
        return { kind: "browser", write: async (blob, signal) => {
                checkExportCancelled(signal);
                const stream = await handle.createWritable();
                try {
                    checkExportCancelled(signal);
                    await stream.write(blob);
                    checkExportCancelled(signal);
                    await stream.close();
                    return "saved";
                }
                catch (cause) {
                    try {
                        await stream.abort();
                    }
                    catch (abortCause) {
                        throw new AggregateError([cause, abortCause], "NOTE_HTML_BROWSER_WRITE_FAILED：HTML 写入和取消写入均未完成。", { cause });
                    }
                    throw cause;
                }
            } };
    }
    return { kind: "browser", write: async (blob, signal) => {
            checkExportCancelled(signal);
            const url = URL.createObjectURL(blob);
            const anchor = document.createElement("a");
            anchor.href = url;
            anchor.download = name;
            anchor.click();
            setTimeout(() => URL.revokeObjectURL(url), 60000);
            return "download-started";
        } };
}
