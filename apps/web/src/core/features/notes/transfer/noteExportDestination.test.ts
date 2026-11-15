// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { chooseNoteExportDestination, noteExportFileName } from "./noteExportDestination";
const mocks = vi.hoisted(() => ({ mode: "desktop", save: vi.fn() }));
vi.mock("../../../runtime/appRuntime", () => ({ getAppRuntime: () => ({ mode: mocks.mode }) }));
vi.mock("../../../runtime/platformDialog", () => ({ save: mocks.save }));
vi.mock("../../../runtime/platformPath", () => ({ downloadDir: async () => "/Downloads", join: async (...parts: string[]) => parts.join("/") }));
beforeEach(() => { mocks.mode = "desktop"; mocks.save.mockReset(); });
afterEach(() => { delete (window as Window & {
    showSaveFilePicker?: unknown;
}).showSaveFilePicker; vi.restoreAllMocks(); });
it.each(["pdf", "html"] as const)("chooses a native %s path before any binary resource is read", async (format) => {
    const path = `/Downloads/当前笔记.${format}`;
    mocks.save.mockResolvedValue(path);
    expect(await chooseNoteExportDestination(noteExportFileName("当前笔记", format), format)).toEqual({ kind: "desktop", path });
    expect(mocks.save).toHaveBeenCalledWith({ title: `导出笔记 ${format.toUpperCase()}`, defaultPath: path, filters: [{ name: format.toUpperCase(), extensions: [format] }] });
});
it("cancels native save without an export destination", async () => {
    mocks.save.mockResolvedValue(null);
    expect(await chooseNoteExportDestination("标题.html", "html")).toBeUndefined();
});
it("refuses browser PDF without opening a picker", async () => {
    mocks.mode = "preview";
    const picker = vi.fn();
    Object.defineProperty(window, "showSaveFilePicker", { configurable: true, value: picker });
    await expect(chooseNoteExportDestination("标题.pdf", "pdf")).rejects.toThrow("NOTE_PDF_DESKTOP_ONLY");
    expect(picker).not.toHaveBeenCalled();
});
it("writes the worker Blob directly and aborts a cancelled staged browser write", async () => {
    mocks.mode = "browser";
    const signal = new AbortController();
    const stream = { write: vi.fn(async () => { signal.abort(); }), close: vi.fn(), abort: vi.fn() };
    const picker = vi.fn(async () => ({ createWritable: async () => stream }));
    Object.defineProperty(window, "showSaveFilePicker", { configurable: true, value: picker });
    const target = await chooseNoteExportDestination("标题.html", "html");
    if (!target || target.kind !== "browser")
        throw new Error("browser destination missing");
    const blob = new Blob(["<!doctype html>"], { type: "text/html" });
    await expect(target.write(blob, signal.signal)).rejects.toMatchObject({ name: "AbortError" });
    expect(stream.write).toHaveBeenCalledWith(blob);
    expect(stream.close).not.toHaveBeenCalled();
    expect(stream.abort).toHaveBeenCalledOnce();
});
it.each(["pdf", "html"] as const)("keeps %s names safe and Chinese readable", format => {
    expect(noteExportFileName("中文标题", format)).toBe(`中文标题.${format}`);
    expect(noteExportFileName(" ", format)).toBe(`无标题.${format}`);
    expect(noteExportFileName("../研究/记录:1", format)).toBe(`-研究-记录-1.${format}`);
    expect(noteExportFileName("CON", format)).toBe(`_CON.${format}`);
    expect(new TextEncoder().encode(noteExportFileName("长".repeat(300), format)).length).toBeLessThanOrEqual(205);
});
