import { openPath as nativeOpenPath, revealItemInDir as nativeReveal } from "@tauri-apps/plugin-opener";
import { isAppPreview } from "./appRuntime";
import { openBrowserPage } from "./browserAdapter";
// Authentication is still simulated in preview; user web links use the browser adapter.
export const openUrl = (url: string | URL): Promise<void> => isAppPreview() ? Promise.resolve() : openBrowserPage(String(url));
export const openPath: typeof nativeOpenPath = (...args) => isAppPreview() ? Promise.resolve() : nativeOpenPath(...args);
export const revealItemInDir: typeof nativeReveal = (...args) => isAppPreview() ? Promise.resolve() : nativeReveal(...args);
