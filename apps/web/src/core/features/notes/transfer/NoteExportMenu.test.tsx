// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { ActionMenu } from "../../../components/ActionMenu";
import { NoteExportMenu } from "./NoteExportMenu";
const mocks = vi.hoisted(() => ({ mode: "desktop", start: vi.fn(), invoke: vi.fn() }));
vi.mock("./noteDocumentExport", () => ({ startNoteDocumentExport: mocks.start }));
vi.mock("../../../runtime/appRuntime", () => ({ getAppRuntime: () => ({ mode: mocks.mode }) }));
vi.mock("../../../runtime/platformCore", () => ({ invoke: mocks.invoke }));
vi.mock("../NotesClient", () => ({ useNotesClient: () => ({ gateway: { readNoteAsset: async () => new Uint8Array() } }) }));
afterEach(() => { vi.unstubAllGlobals(); mocks.start.mockReset(); mocks.invoke.mockReset(); });
it.each([["desktop", "pdf", true], ["preview", "pdf", false], ["preview", "html", true]] as const)("offers %s %s only when supported", async (mode, format, enabled) => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    mocks.mode = mode;
    mocks.invoke.mockResolvedValue({ pdf: true });
    mocks.start.mockResolvedValue(undefined);
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    try {
        await act(async () => root.render(<ActionMenu label="页面操作" className="test-menu" trigger="更多"><NoteExportMenu pageId="page" title="标题"/></ActionMenu>));
        await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="页面操作"]')!.click());
        await vi.waitFor(() => expect(document.querySelector('[role="menuitem"][aria-label="导出"]')).not.toBeNull());
        await act(async () => document.querySelector<HTMLButtonElement>('[role="menuitem"][aria-label="导出"]')!.click());
        await vi.waitFor(() => expect(document.querySelector('[role="menu"][aria-label="导出"]')).not.toBeNull());
        const menu = document.querySelector('[role="menu"][aria-label="导出"]')!;
        expect([...menu.querySelectorAll('[role="menuitem"]')].map(item => item.textContent)).toEqual(["导出 PDF", "导出 HTML"]);
        const button = [...menu.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === `导出 ${format.toUpperCase()}`)!;
        await vi.waitFor(() => expect(button.disabled).toBe(!enabled));
        if (enabled) {
            await act(async () => button.click());
            expect(mocks.start).toHaveBeenCalledWith(format, "page", "标题", expect.any(Function));
        }
        else {
            expect(mocks.start).not.toHaveBeenCalled();
            expect(mocks.invoke).not.toHaveBeenCalled();
        }
    }
    finally {
        await act(async () => root.unmount());
        host.remove();
    }
});
