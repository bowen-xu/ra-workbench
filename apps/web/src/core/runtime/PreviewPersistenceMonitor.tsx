import { useEffect } from "react";
import { notify } from "../services/notifications";
import { storagePersistence } from "./storagePersistence";
import { PreviewStorageError } from "./previewIndexedDatabase";

/** Also covers synchronous preference/draft writes, which have no async gateway receipt. */
export function PreviewPersistenceMonitor({storage}: {readonly storage: Storage}): null {
  useEffect(() => {
    const port = storagePersistence(storage);
    if (!port) return;
    const id = "preview-indexeddb-persistence";
    let timer: ReturnType<typeof setTimeout> | undefined;
    const update = () => {
      if (timer !== undefined) { clearTimeout(timer); timer = undefined; }
      if (port.error) {
        const conflict = port.error instanceof PreviewStorageError && port.error.code === "preview_storage_conflict";
        notify.error(port.error.message, {id, dismissible: false, actions: conflict ? [{id: "reload", label: "刷新核对", onClick: () => window.location.reload()}] : [{id: "retry", label: "重试保存", onClick: async () => {
          await port.retry();
          if (port.resetting) window.location.reload();
        }}]});
      } else if (port.pending) {
        timer = setTimeout(() => notify.loading("正在保存预览数据…", {id}), 300);
      } else notify.dismiss(id);
    };
    const leaving = (event: BeforeUnloadEvent) => {
      if (port.pending || port.error) { event.preventDefault(); event.returnValue = ""; }
    };
    const unsubscribe = port.subscribe(update);
    window.addEventListener("beforeunload", leaving); update();
    return () => { unsubscribe(); if (timer !== undefined) clearTimeout(timer); notify.dismiss(id); window.removeEventListener("beforeunload", leaving); };
  }, [storage]);
  return null;
}
