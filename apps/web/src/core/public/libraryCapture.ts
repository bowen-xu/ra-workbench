/** External capture signals and native file-drop transport; import policy stays in the plugin. */
export { listenBrowserCaptures } from "../services/browserConnectorGateway";
export type { BrowserCaptureEvent } from "../services/browserConnectorGateway";
export { subscribeWindowFileDrop } from "../runtime/windowFileDrop";
export { desktopDropClientPosition } from "../runtime/desktopDragDrop";
export type { WindowFileDropEvent, FileDropPosition } from "../contracts/fileDrop";
