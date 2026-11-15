import { isAppPreview } from "./appRuntime";
/** Browser file pickers are side effects too: preview supplies synthetic bytes. */
export function chooseAppFile(input: HTMLInputElement | null, receive: (file: File) => void, kind: "markdown" | "asset"): void {
  if (!isAppPreview()) { input?.click(); return; }
  receive(kind === "markdown" ? new File(["# 合成导入笔记\n\n这些内容来自预览样本。"], "preview-note.md", { type: "text/markdown" }) : new File(["合成附件"], "preview-asset.txt", { type: "text/plain" }));
}
