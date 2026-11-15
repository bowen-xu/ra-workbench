// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { startNoteDocumentExport } from "./noteDocumentExport";
const mocks = vi.hoisted(() => ({ mode: "preview", choose: vi.fn(), prepare: vi.fn(), build: vi.fn(), invoke: vi.fn(), write: vi.fn(), loading: vi.fn(), dismiss: vi.fn(), success: vi.fn(), error: vi.fn(), info: vi.fn(), update: vi.fn(), warning: vi.fn() }));
vi.mock("../../../runtime/appRuntime", () => ({ getAppRuntime: () => ({ mode: mocks.mode }) }));
vi.mock("../../../runtime/platformCore", () => ({ invoke: mocks.invoke }));
vi.mock("../../../services/notifications", () => ({ notify: { loading: mocks.loading, dismiss: mocks.dismiss, success: mocks.success, error: mocks.error, info: mocks.info, update: mocks.update, warning: mocks.warning } }));
vi.mock("./noteExportDestination", async () => ({ ...await vi.importActual<typeof import("./noteExportDestination")>("./noteExportDestination"), chooseNoteExportDestination: mocks.choose }));
vi.mock("./prepareNoteExportInput", () => ({ prepareNoteExportInput: mocks.prepare }));
vi.mock("./browserNoteExport", () => ({ buildBrowserNoteExport: mocks.build }));
beforeEach(() => {
    for (const value of Object.values(mocks))
        if (typeof value === "function")
            value.mockReset();
    mocks.mode = "preview";
    mocks.loading.mockReturnValue("notice");
    document.body.innerHTML = '<div data-note-pdf-page-id="page"><p>正文</p></div>';
    mocks.choose.mockResolvedValue({ kind: "browser", write: mocks.write });
    mocks.prepare.mockResolvedValue({ template: "<!doctype html>", resources: [] });
    mocks.build.mockResolvedValue(new Blob(["<!doctype html>"]));
    mocks.write.mockResolvedValue("download-started");
});
afterEach(() => { document.body.innerHTML = ""; vi.restoreAllMocks(); });
const read = async () => new Uint8Array();
function stop() { const options = mocks.loading.mock.calls[0]![1] as {
    actions: {
        onClick: () => Promise<void>;
    }[];
}; return options.actions[0]!.onClick(); }
it("keeps browser PDF disabled even when called outside the menu", async () => {
    await startNoteDocumentExport("pdf", "page", "标题", read);
    expect(mocks.choose).not.toHaveBeenCalled();
    expect(mocks.error).toHaveBeenCalledWith(expect.stringContaining("NOTE_PDF_DESKTOP_ONLY"));
});
it("does not prepare resources or create a job after save cancellation", async () => {
    mocks.choose.mockResolvedValue(undefined);
    await startNoteDocumentExport("html", "page", "标题", read);
    expect(mocks.prepare).not.toHaveBeenCalled();
    expect(mocks.loading).not.toHaveBeenCalled();
    expect(mocks.build).not.toHaveBeenCalled();
});
it("stops the real task through its persistent notification and never publishes", async () => {
    mocks.build.mockImplementation((_input, _read, signal: AbortSignal) => new Promise((_resolve, reject) => signal.addEventListener("abort", () => reject(new DOMException("stopped", "AbortError")), { once: true })));
    const task = startNoteDocumentExport("html", "page", "标题", read);
    await vi.waitFor(() => expect(mocks.build).toHaveBeenCalledOnce());
    await stop();
    await task;
    expect(mocks.write).not.toHaveBeenCalled();
    expect(mocks.info).toHaveBeenCalledWith("HTML 导出已停止。");
    expect(mocks.error).not.toHaveBeenCalled();
    expect(mocks.dismiss).toHaveBeenCalledWith("notice");
});
it("hands only a small template to native and releases job ownership on completion", async () => {
    mocks.mode = "desktop";
    mocks.choose.mockResolvedValue({ kind: "desktop", path: "/Downloads/note.pdf" });
    mocks.invoke.mockResolvedValue(undefined);
    await startNoteDocumentExport("pdf", "page", "标题", read);
    expect(mocks.invoke).toHaveBeenCalledWith("run_note_document_export", { jobId: expect.any(String), destination: "/Downloads/note.pdf", format: "pdf", input: { template: "<!doctype html>", resources: [] } });
    expect(mocks.invoke).toHaveBeenCalledWith("release_note_document_export", { jobId: expect.any(String) });
    expect(mocks.build).not.toHaveBeenCalled();
    expect(mocks.success).toHaveBeenCalledWith("PDF 已导出。");
});
it("treats a completed native publication as saved when a late stop loses the race", async () => {
    mocks.mode = "desktop";
    mocks.choose.mockResolvedValue({ kind: "desktop", path: "/Downloads/note.pdf" });
    let finish: () => void = () => { };
    mocks.invoke.mockImplementation((command: string) => command === "run_note_document_export" ? new Promise<void>(resolve => { finish = resolve; }) : Promise.resolve(command === "cancel_note_document_export" ? false : undefined));
    const task = startNoteDocumentExport("pdf", "page", "标题", read);
    await vi.waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("run_note_document_export", expect.any(Object)));
    await stop();
    finish();
    await task;
    expect(mocks.success).toHaveBeenCalledWith("PDF 已导出。");
    expect(mocks.info).not.toHaveBeenCalledWith("PDF 导出已停止。");
});
