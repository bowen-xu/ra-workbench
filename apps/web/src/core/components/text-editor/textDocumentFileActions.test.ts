import { beforeEach, expect, it, vi } from "vitest";
import { revealTextDocument, saveTextDocumentAs } from "./textDocumentFileActions";
const ports = vi.hoisted(() => ({ invoke: vi.fn(), save: vi.fn(), reveal: vi.fn(), runtime: { mode: "desktop" } }));
vi.mock("../../runtime/appRuntime", () => ({ getAppRuntime: () => ports.runtime }));
vi.mock("../../runtime/platformCore", () => ({ invoke: ports.invoke }));
vi.mock("../../runtime/platformDialog", () => ({ save: ports.save }));
vi.mock("../../runtime/platformOpener", () => ({ revealItemInDir: ports.reveal }));
vi.mock("../../services/notifications", () => ({ notify: { info: vi.fn() } }));
beforeEach(() => { vi.clearAllMocks(); ports.runtime.mode = "desktop"; });
it("resolves builtin storage before revealing and never treats a source address as a file", async () => {
  ports.invoke.mockResolvedValue("/data/builtin-documents.v1.json");
  await revealTextDocument("system:structured-task", "source://agents/prompts/structured-task.md");
  expect(ports.invoke).toHaveBeenCalledWith("system_documents_location", { id: "structured-task" });
  expect(ports.reveal).toHaveBeenCalledWith("/data/builtin-documents.v1.json");
  await expect(revealTextDocument("unknown", "source://assets/file")).rejects.toThrow("没有可显示");
});
it("exports current text through the save dialog and cancellation writes nothing", async () => {
  ports.save.mockResolvedValueOnce(null).mockResolvedValueOnce("/exports/copy.md");
  await saveTextDocumentAs("source://agents/prompts/task.md", "unsaved text");
  expect(ports.invoke).not.toHaveBeenCalled();
  await saveTextDocumentAs("source://agents/prompts/task.md", "unsaved text");
  expect(ports.invoke).toHaveBeenCalledWith("text_document_export", { destination: "/exports/copy.md", content: "unsaved text" });
});
it("does not call native operations from browser or preview", async () => {
  ports.runtime.mode = "preview";
  await expect(revealTextDocument("system:structured-task", "source://agents/prompts/structured-task.md")).rejects.toThrow("桌面应用");
  await saveTextDocumentAs("file.md", "draft");
  expect(ports.invoke).not.toHaveBeenCalled(); expect(ports.save).not.toHaveBeenCalled(); expect(ports.reveal).not.toHaveBeenCalled();
});
