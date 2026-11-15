import {isWritingNotePath} from "@research-assistant/writing";
import {createCatalogStores} from "../notes/gateways/browserNoteCatalogStore";
import {writingFileKey} from "./browserWritingRecords";
import {decodeWritingFile, decodeWritingProject} from "./writingRecordCodec";
import {changeWritingCatalogCounts, emptyWritingCatalogState, type WritingCatalogWork} from "./writingCatalogState";

export function createWritingCatalogStores(database: IDBDatabase): void {
  createCatalogStores(database);
  const work = database.createObjectStore("writingCatalogFiles");
  work.createIndex("project","manuscriptId");
  work.createIndex("identity","id",{unique: true});
  work.createIndex("statusState",["status","state"]);
}

/** Schema 3 registers legacy file headers only. Reading/parsing old bodies is
 * a separate explicit preparation operation, never an upgrade side effect. */
export function upgradeWritingCatalog(tx: IDBTransaction, failed: (error: Error) => void): void {
  let state = emptyWritingCatalogState();
  const request = tx.objectStore("writingFiles").openCursor();
  const abort = () => { failed(Object.assign(new Error("writing_note_catalog_upgrade_invalid：写作文件元数据无法登记；升级已回滚，原文件已保留。"), {code: "writing_note_catalog_upgrade_invalid"})); tx.abort(); };
  request.onsuccess = () => {
    const entry = request.result;
    if (!entry) { tx.objectStore("catalogState").put(state,"current"); return; }
    try {
      const file = decodeWritingFile(entry.value);
      if (!isWritingNotePath(file.relativePath)) { entry.continue(); return; }
      const project = tx.objectStore("writingProjects").get(file.manuscriptId);
      project.onsuccess = () => {
        try {
          const owner = decodeWritingProject(project.result).manuscript;
          if (owner.id !== file.manuscriptId || entry.key !== writingFileKey(file.manuscriptId,file.relativePath)) { abort(); return; }
          const work: WritingCatalogWork = {manuscriptId: owner.id,path: file.relativePath,sha256: file.sha256,status: owner.status,state: "pending"};
          tx.objectStore("writingCatalogFiles").add(work,entry.key);
          state = changeWritingCatalogCounts(state,undefined,work); entry.continue();
        } catch { abort(); }
      };
    } catch { abort(); }
  };
}
