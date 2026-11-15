import { getCurrentWindow } from "@tauri-apps/api/window";
import type { WindowFileDropEvent } from "../contracts/fileDrop";

/** Keep the original Tauri window subscription and unlisten ownership. */
export function subscribeWindowFileDrop(listener: (event: WindowFileDropEvent) => void): Promise<() => void> {
  return getCurrentWindow().onDragDropEvent(event => listener(event.payload));
}
