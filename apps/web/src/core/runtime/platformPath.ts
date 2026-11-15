import { downloadDir as nativeDownloadDir, join as nativeJoin } from "@tauri-apps/api/path";
import { isAppPreview } from "./appRuntime";
export const downloadDir: typeof nativeDownloadDir = () => isAppPreview() ? Promise.resolve("mock://downloads") : nativeDownloadDir();
export const join: typeof nativeJoin = (...parts) => isAppPreview() ? Promise.resolve(parts.join("/")) : nativeJoin(...parts);
