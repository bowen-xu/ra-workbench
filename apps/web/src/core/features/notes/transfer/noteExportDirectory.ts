import {hasPlatformCapabilities} from "../../../runtime/appRuntime";
import {open} from "../../../runtime/platformDialog";

export async function chooseNoteExportDirectory(): Promise<string | undefined> {
  if (!hasPlatformCapabilities()) throw new Error("页面树与附件导出请在桌面应用中使用");
  const targetDirectory = await open({ directory: true, multiple: false, title: "选择笔记导出目录" });
  return targetDirectory ?? undefined;
}
