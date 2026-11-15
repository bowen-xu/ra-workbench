import {isWritingNotePath, type ManuscriptFile} from "@research-assistant/writing";
import {noteSearchGrams, type NoteCatalogHeader, type NoteCatalogRuntime} from "@research-assistant/data/notes";
import {catalogRequest, type BrowserCatalogBody} from "../notes/gateways/browserNoteCatalogStore";
import {catalogPostingUpdater} from "../notes/gateways/browserCatalogPostings";
import {writingFileKey, type WritingRecordChange} from "./browserWritingRecords";
import {decodeWritingBody, decodeWritingFile, decodeWritingProject, type StoredWritingProject} from "./writingRecordCodec";
import {projectWritingCatalogContent, projectWritingCatalogHeader, writingCatalogId, type WritingCatalogContent} from "./writingCatalogProjection";
import {changeWritingCatalogCounts, refreshWritingCatalogIssues, useWritingCatalogRuntime, writingCatalogState, type WritingCatalogWork} from "./writingCatalogState";
import {catalogTransactionDone} from "../notes/gateways/browserNoteCatalogStore";
import {loadBrowserNoteCatalogRuntime} from "../notes/gateways/browserNoteCatalogRuntime";

function fileIdentity(key: string): readonly [string,string] {
  const value: unknown = JSON.parse(key);
  if (!Array.isArray(value) || value.length !== 2 || value.some(part => typeof part !== "string")) throw new Error("writing_note_catalog_file_key_invalid");
  return value as [string,string];
}
export function changesWritingFileCatalog(changes: readonly WritingRecordChange[]): boolean {
  return changes.some(change => (change.store === "writingFiles" || change.store === "writingBodies") && isWritingNotePath(fileIdentity(change.key)[1]));
}
/** Determine affected owner labels/lifecycles without reading any file body. */
export async function changedWritingCatalogProjects(tx: IDBTransaction, changes: readonly WritingRecordChange[]): Promise<readonly string[]> {
  const result: string[] = [];
  for (const change of changes) if (change.store === "writingProjects") {
    const raw = await catalogRequest(tx.objectStore("writingProjects").get(change.key));
    const before = raw === undefined ? undefined : decodeWritingProject(raw).manuscript;
    const after = change.value === undefined ? undefined : decodeWritingProject(change.value).manuscript;
    if (before && (before.title !== after?.title || before.status !== after?.status)) result.push(change.key);
  }
  return result;
}
export async function writingCatalogRuntimeForChanges(database: IDBDatabase, changes: readonly WritingRecordChange[]): Promise<NoteCatalogRuntime | undefined> {
  if (changesWritingFileCatalog(changes)) return loadBrowserNoteCatalogRuntime();
  if (!changes.some(change => change.store === "writingProjects")) return;
  const tx = database.transaction(["writingProjects","writingCatalogFiles"],"readonly"), done = catalogTransactionDone(tx);
  try {
    const projects = await changedWritingCatalogProjects(tx,changes);
    const keys = await Promise.all(projects.map(id => catalogRequest(tx.objectStore("writingCatalogFiles").index("project").getKey(id))));
    await done;
    if (keys.some(key => key !== undefined)) return loadBrowserNoteCatalogRuntime();
  } catch (reason) { await done.catch(() => undefined); throw reason; }
}

/** All requests use the caller's canonical CAS transaction. A file save supplies
 * its received bytes; metadata edits reuse existing body/search projections. */
export async function applyWritingCatalogChanges(tx: IDBTransaction, changes: readonly WritingRecordChange[], runtime: NoteCatalogRuntime,
  prepared: ReadonlyMap<string,WritingCatalogContent> = new Map()): Promise<void> {
  let state = useWritingCatalogRuntime(await writingCatalogState(tx),runtime);
  const incoming = new Map(changes.map(change => [`${change.store}:${change.key}`,change]));
  const after = async (store: "writingProjects" | "writingFiles", key: string): Promise<unknown> => {
    const change = incoming.get(`${store}:${key}`);
    return change ? change.value : catalogRequest(tx.objectStore(store).get(key));
  };
  const affected = new Set(changes.filter(change => change.store === "writingFiles" || change.store === "writingBodies")
    .map(change => change.key).filter(key => isWritingNotePath(fileIdentity(key)[1])));
  const workStore = tx.objectStore("writingCatalogFiles"), headers = tx.objectStore("catalogHeaders"), bodies = tx.objectStore("catalogBodies");
  for (const id of await changedWritingCatalogProjects(tx,changes)) {
    const keys = await catalogRequest(workStore.index("project").getAllKeys(id));
    for (const key of keys) affected.add(String(key));
  }
  if (!affected.size) return;
  const old = new Map<string,WritingCatalogWork>(), files = new Map<string,ManuscriptFile>(), projects = new Map<string,StoredWritingProject | undefined>();
  for (const key of affected) {
    const previous = await catalogRequest(workStore.get(key)) as WritingCatalogWork | undefined;
    if (previous) old.set(key,previous);
    const [id,path] = fileIdentity(key), raw = await after("writingFiles",key);
    if (!projects.has(id)) { const project = await after("writingProjects",id); projects.set(id,project === undefined ? undefined : decodeWritingProject(project)); }
    if (raw !== undefined && projects.get(id)) {
      const file = decodeWritingFile(raw);
      if (file.manuscriptId !== id || file.relativePath !== path) throw new Error("writing_note_catalog_file_key_mismatch");
      files.set(key,file);
    }
  }
  const removed = new Map([...old].filter(([key]) => !files.has(key)));
  const transfers = new Map([...removed.values()].filter(work => work.state === "ready").map(work => [JSON.stringify([work.manuscriptId,work.sha256]),work]));
  // Remove old file ownership before transferring its stable note identity.
  for (const [key, previous] of removed) { workStore.delete(key); state = changeWritingCatalogCounts(state,previous,undefined); }
  const plans: {key: string;work: WritingCatalogWork;content?: {readonly noteId: string;readonly text: string}}[] = [];
  const claimed = new Set<string>();
  for (const [key,file] of files) {
    const owner = projects.get(file.manuscriptId)!.manuscript;
    const prior = old.get(key) ?? transfers.get(JSON.stringify([file.manuscriptId,file.sha256]));
    let id = prior?.state === "ready" && prior.sha256 === file.sha256 ? prior.id : undefined;
    let content: WritingCatalogContent | undefined;
    const bodyChange = incoming.get(`writingBodies:${key}`);
    const supplied = bodyChange?.value === undefined ? undefined : decodeWritingBody(bodyChange.value);
    if (supplied && (supplied.sha256 !== file.sha256 || supplied.relativePath !== file.relativePath))
      throw new Error("writing_note_catalog_body_mismatch：正文与本次文件登记不一致，写入未提交。");
    if (bodyChange && bodyChange.value === undefined) {
      id = undefined; content = {error: "writing_note_document_missing：写作笔记正文缺失；已有记录已保留。"};
    }
    if (!id) {
      content ??= prepared.get(key);
      if (!content && supplied !== undefined) {
        content = projectWritingCatalogContent(supplied.content);
      }
      if (content && "noteId" in content) id = writingCatalogId(owner.id,content.noteId);
    }
    let cause = content && "error" in content ? content.error : undefined;
    if (id) {
      const existing = await catalogRequest(headers.get(id)) as NoteCatalogHeader | undefined;
      if (claimed.has(id) || existing?.item.target.kind === "writing" && existing.item.target.path !== file.relativePath
        && !removed.has(writingFileKey(file.manuscriptId,existing.item.target.path))) {
        cause = "writing_note_identity_duplicate：同一项目的写作笔记标识重复；原文件均已保留。"; id = undefined;
      } else claimed.add(id);
    }
    const work: WritingCatalogWork = {manuscriptId: owner.id,path: file.relativePath,sha256: file.sha256,status: owner.status,
      state: cause ? "error" : id ? "ready" : "pending", ...(cause ? {cause} : {}), ...(id ? {id} : {})};
    plans.push({key,work,...(content && "noteId" in content && id ? {content} : {})});
  }
  const updateGrams = catalogPostingUpdater(tx);
  for (const id of new Set([...old.values()].flatMap(work => work.id && !claimed.has(work.id) ? [work.id] : []))) {
    const header = await catalogRequest(headers.get(id)) as NoteCatalogHeader | undefined;
    if (!header) continue;
    const body = await catalogRequest(bodies.get(id)) as BrowserCatalogBody | undefined;
    await updateGrams(header.ordinal * 2,new Set(body?.grams),new Set());
    await updateGrams(header.ordinal * 2 + 1,noteSearchGrams(header.metadataText),new Set()); headers.delete(id); bodies.delete(id);
  }
  for (const plan of plans) {
    const {work,key} = plan;
    if (work.id) {
      const previous = await catalogRequest(headers.get(work.id)) as NoteCatalogHeader | undefined;
      const ordinal = previous?.ordinal ?? state.nextOrdinal;
      if (!previous) state = {...state,nextOrdinal: ordinal + 1};
      if (plan.content) {
        const before = await catalogRequest(bodies.get(work.id)) as BrowserCatalogBody | undefined;
        const text = plan.content.text, foldedText = runtime.fold(text), grams = noteSearchGrams(foldedText);
        await updateGrams(ordinal * 2,new Set(before?.grams),grams);
        bodies.put({text,foldedText,grams: [...grams]} satisfies BrowserCatalogBody,work.id);
      }
      const header = projectWritingCatalogHeader(files.get(key)!,projects.get(work.manuscriptId)!.manuscript,work.id,ordinal,runtime);
      await updateGrams(ordinal * 2 + 1,noteSearchGrams(previous?.metadataText ?? ""),noteSearchGrams(header.metadataText)); headers.put(header);
    }
    state = changeWritingCatalogCounts(state,removed.has(key) ? undefined : old.get(key),work); workStore.put(work,key);
  }
  updateGrams.flush();
  state = await refreshWritingCatalogIssues(tx,state);
  tx.objectStore("catalogState").put({...state,generation: state.generation + 1},"current");
}
