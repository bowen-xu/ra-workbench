import { getAppRuntime } from "../../../runtime/appRuntime";
import { invoke } from "../../../runtime/platformCore";
import { notify } from "../../../services/notifications";
import { captureNoteExport } from "./noteExportSource";
import { chooseNoteExportDestination, noteExportFileName, type NoteExportFormat } from "./noteExportDestination";
import { prepareNoteExportInput } from "./prepareNoteExportInput";
import { buildBrowserNoteExport } from "./browserNoteExport";
import { checkExportCancelled, yieldExport } from "./noteExportInput";
import "./NotePdfExport.css";
interface ExportTask {
    readonly id: string;
    readonly controller: AbortController;
    notice: string | number;
    native: boolean;
    rendering: boolean;
}
const tasks = new Map<string, ExportTask>();
function causeText(cause: unknown): string { return cause instanceof Error ? cause.message : String(cause); }
function cancelled(cause: unknown): boolean { return cause instanceof DOMException && cause.name === "AbortError" || causeText(cause).includes("NOTE_EXPORT_CANCELLED"); }
async function stop(task: ExportTask): Promise<void> {
    if (task.controller.signal.aborted)
        return;
    if (task.native) {
        const accepted = await invoke<boolean>("cancel_note_document_export", { jobId: task.id });
        if (!accepted && task.rendering) {
            notify.update(task.notice, { message: "正在完成保存…", actions: [] });
            return;
        }
    }
    task.controller.abort();
    notify.update(task.notice, { message: "正在停止导出…", actions: [] });
}
/** Job lifetime is independent of menus and editor navigation. The snapshot
 * cannot change the note, undo history, save owner or HBS review decisions. */
export async function startNoteDocumentExport(format: NoteExportFormat, pageId: string, title: string, readAsset: (id: string) => Promise<Uint8Array>): Promise<void> {
    const label = format.toUpperCase();
    let task: ExportTask | undefined;
    try {
        if (format === "pdf" && getAppRuntime().mode !== "desktop")
            throw new Error("NOTE_PDF_DESKTOP_ONLY：PDF 导出仅在桌面应用可用。");
        if (tasks.size >= 2)
            throw new Error("NOTE_EXPORT_BUSY：已有两个导出任务，请先完成或停止其中一个。");
        const frozen = captureNoteExport(pageId, title);
        const destination = await chooseNoteExportDestination(noteExportFileName(frozen.title, format), format);
        if (!destination)
            return;
        const id = crypto.randomUUID();
        task = { id, controller: new AbortController(), notice: "", native: false, rendering: false };
        const current = task;
        current.notice = notify.loading(`正在导出 ${label}…`, { actions: [{ id: "stop-export", label: "停止导出", onClick: () => stop(current) }] });
        tasks.set(id, current);
        await yieldExport(current.controller.signal);
        if (destination.kind === "desktop") {
            await invoke("begin_note_document_export", { jobId: id });
            current.native = true;
            if (current.controller.signal.aborted) {
                await invoke("cancel_note_document_export", { jobId: id });
                checkExportCancelled(current.controller.signal);
            }
        }
        const input = await prepareNoteExportInput(frozen.snapshot, frozen.title, current.controller.signal);
        checkExportCancelled(current.controller.signal);
        let result: "saved" | "download-started";
        if (destination.kind === "desktop") {
            current.rendering = true;
            await invoke("run_note_document_export", { jobId: id, destination: destination.path, format, input });
            result = "saved";
        }
        else {
            const blob = await buildBrowserNoteExport(input, readAsset, current.controller.signal);
            checkExportCancelled(current.controller.signal);
            current.rendering = true;
            result = await destination.write(blob, current.controller.signal);
        }
        notify.dismiss(current.notice);
        if (result === "saved")
            notify.success(`${label} 已导出。`);
        else
            notify.info(`已开始下载 ${label}。`);
    }
    catch (cause) {
        if (task)
            notify.dismiss(task.notice);
        if (cancelled(cause))
            notify.info(`${label} 导出已停止。`);
        else
            notify.error(`NOTE_${label}_EXPORT_FAILED：${causeText(cause)}${task ? `；task=${task.id}` : ""}`);
    }
    finally {
        if (task) {
            tasks.delete(task.id);
            if (task.native) {
                try {
                    await invoke("release_note_document_export", { jobId: task.id });
                }
                catch (cause) {
                    notify.warning(`NOTE_EXPORT_CLEANUP_FAILED：${causeText(cause)}；task=${task.id}`);
                }
            }
        }
    }
}
