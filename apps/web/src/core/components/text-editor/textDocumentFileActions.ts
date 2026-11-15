import { getAppRuntime } from "../../runtime/appRuntime";
import { invoke } from "../../runtime/platformCore";
import { save } from "../../runtime/platformDialog";
import { revealItemInDir } from "../../runtime/platformOpener";
import { notify } from "../../services/notifications";

export async function revealTextDocument(id: string, path: string): Promise<void> {
  if (getAppRuntime().mode !== "desktop") throw new Error("请在桌面应用中使用“在 Finder 中显示”。");
  const physicalPath = id.startsWith("system:")
    ? await invoke<string>("system_documents_location", { id: id.slice(7) })
    : /^(\/|[a-z]:[\\/])/i.test(path) ? path : undefined;
  if (!physicalPath) throw new Error("此来源没有可显示的本地文件。");
  await revealItemInDir(physicalPath);
}

export async function saveTextDocumentAs(path: string, content: string): Promise<void> {
  const name = path.split(/[\\/]/).at(-1) || "document.txt";
  if (getAppRuntime().mode === "preview") { notify.info("预览：已模拟另存为当前文本。"); return; }
  if (getAppRuntime().mode === "desktop") {
    const destination = await save({ title: "另存为", defaultPath: name });
    if (!destination) return;
    await invoke("text_document_export", { destination, content });
  } else {
    const url = URL.createObjectURL(new Blob([content], { type: "text/plain;charset=utf-8" }));
    const anchor = document.createElement("a"); anchor.href = url; anchor.download = name; anchor.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
}
