import { classifyManuscriptFile } from "@research-assistant/writing";
import { writingFileKey } from "./browserWritingRecords";
import { decodeWritingBody, decodeWritingDraft, decodeWritingProject, decodeWritingSession, record, text } from "./writingRecordCodec";

/** Schema v1 stored writing as one entry. Split once in the upgrade transaction;
 * an invalid source aborts the whole upgrade and leaves the original untouched. */
export function upgradeWritingRecords(tx: IDBTransaction, failed: (reason: Error) => void, completed?: () => void): void {
  tx.addEventListener("error", event => {
    const reason = event.target instanceof IDBRequest ? event.target.error : tx.error;
    failed(Object.assign(new Error(`writing_storage_corrupt：旧写作记录升级事务失败，原记录已保留。${reason?.name ?? "IndexedDBError"}: ${reason?.message ?? ""}`, {cause: reason}), {code: "writing_storage_corrupt"}));
  });
  const entries = tx.objectStore("entries"), request = entries.get("writing-files");
  request.onsuccess = () => {
    if (request.result === undefined) { completed?.(); return; }
    let stage = "snapshot";
    try {
      const state = record(JSON.parse(text(request.result)) as unknown);
      if (!Array.isArray(state.manuscripts) || !Array.isArray(state.sessions) || !Array.isArray(state.drafts)) throw new Error("writing_storage_corrupt");
      stage = "manuscripts";
      for (const pair of state.manuscripts) {
        if (!Array.isArray(pair) || pair.length !== 2) throw new Error("writing_storage_corrupt");
        const id = text(pair[0]), source = record(pair[1]), project = decodeWritingProject(source);
        if (id !== project.manuscript.id || !Array.isArray(source.files)) throw new Error("writing_storage_corrupt");
        const dates = new Map(Array.isArray(source.editedAt) ? source.editedAt as [string, unknown][] : []);
        tx.objectStore("writingProjects").add(project, id);
        for (const item of source.files) {
          if (!Array.isArray(item) || item.length !== 2) throw new Error("writing_storage_corrupt");
          const body = decodeWritingBody(item[1]);
          if (item[0] !== body.relativePath) throw new Error("writing_storage_corrupt");
          const modifiedAt = dates.get(body.relativePath) ?? project.manuscript.updatedAt;
          if (typeof modifiedAt !== "string" || !Number.isFinite(Date.parse(modifiedAt))) throw new Error("writing_storage_corrupt");
          const key = writingFileKey(id, body.relativePath);
          tx.objectStore("writingFiles").add({manuscriptId: id, relativePath: body.relativePath, sha256: body.sha256,
            kind: classifyManuscriptFile(body.relativePath), sizeBytes: new TextEncoder().encode(body.content).length, modifiedAt}, key);
          tx.objectStore("writingBodies").add(body, key);
        }
      }
      stage = "sessions";
      for (const pair of state.sessions) {
        if (!Array.isArray(pair) || pair.length !== 2) throw new Error("writing_storage_corrupt");
        const session = decodeWritingSession(pair[1]);
        if (pair[0] !== session.manuscriptId) throw new Error("writing_storage_corrupt");
        tx.objectStore("writingSessions").add(session, session.manuscriptId);
      }
      stage = "drafts";
      for (const pair of state.drafts) {
        if (!Array.isArray(pair) || pair.length !== 2) throw new Error("writing_storage_corrupt");
        const draft = decodeWritingDraft(pair[1]);
        if (pair[0] !== `${draft.manuscriptId}:${draft.relativePath}`) throw new Error("writing_storage_corrupt");
        tx.objectStore("writingDrafts").add(draft, writingFileKey(draft.manuscriptId, draft.relativePath));
      }
      entries.delete("writing-files");
      completed?.();
    } catch (reason) {
      // JSON.parse can echo private source text. Preserve its category and
      // available numeric location, never its excerpt or the original payload.
      const location = reason instanceof SyntaxError ? reason.message.match(/position\s+\d+|line\s+\d+\s+column\s+\d+/gu)?.join("; ") : undefined;
      failed(Object.assign(new Error(`writing_storage_corrupt：旧写作记录未通过升级校验，原记录已保留。stage=${stage}; cause=${reason instanceof Error ? reason.name : "invalid_record"}${location ? `; ${location}` : ""}`), {code: "writing_storage_corrupt"}));
      tx.abort();
    }
  };
}
