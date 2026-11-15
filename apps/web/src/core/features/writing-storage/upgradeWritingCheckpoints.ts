import {decodeWritingCheckpointRecord} from "@research-assistant/writing";
import {decodeMockState} from "../../public/storage";
import {writingFileKey} from "./browserWritingRecords";

/** v4 only. Existing v1/v2/v3 upgrades keep their deployed definitions. */
export function createWritingCheckpointStore(database: IDBDatabase): void {
  database.createObjectStore("writingCheckpoints").createIndex("manuscriptId", "checkpoint.manuscriptId");
}

/** Move the legacy recovery bundle atomically, preserving every record/revision.
 * Invalid input or a failed add aborts the upgrade; the old bundle survives. */
export function upgradeWritingCheckpoints(tx: IDBTransaction, failed: (reason: Error) => void): void {
  const fail = (reason: unknown, stage: string) => {
    // JSON errors may echo private text. Retain category/stage, never excerpts.
    failed(Object.assign(new Error(`writing_checkpoint_upgrade_failed：检查点升级失败，原数据已保留。stage=${stage}; cause=${reason instanceof Error ? reason.name : "invalid_record"}`), {code: "writing_checkpoint_upgrade_failed"}));
    try { tx.abort(); } catch { /* transaction already aborted */ }
  };
  const entries = tx.objectStore("entries"), store = tx.objectStore("writingCheckpoints");
  const request = entries.get("writing-checkpoints");
  request.onsuccess = () => {
    if (request.result === undefined) return;
    let stage = "bundle";
    try {
      if (typeof request.result !== "string") throw new Error("invalid_bundle");
      const rows: unknown = decodeMockState(request.result);
      if (!(rows instanceof Map)) throw new Error("invalid_bundle");
      for (const [key, value] of rows) {
        stage = "record";
        const row = decodeWritingCheckpointRecord(value);
        stage = "identity";
        if (key !== writingFileKey(row.checkpoint.manuscriptId, row.checkpoint.relativePath)) throw new Error("invalid_identity");
        stage = "copy";
        const add = store.add(row, key);
        add.onerror = () => fail(add.error, "copy");
      }
      const count = store.count();
      count.onsuccess = () => {
        if (count.result !== rows.size) { fail(new Error("count_mismatch"), "verify"); return; }
        entries.delete("writing-checkpoints");
      };
    } catch (reason) { fail(reason, stage); }
  };
}
