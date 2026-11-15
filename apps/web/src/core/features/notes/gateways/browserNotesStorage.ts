import { openNotesDatabase, type BrowserNotesDatabase, type StoredRows, type NoteRead, type NoteWrite } from "./browserNotesDatabase";
import type { NoteMetadataBatchRequest, NoteMetadataBatchReceipt } from "@research-assistant/core/notes";
import {browserHbsRecordKey, type BrowserNoteHbsRecord} from "./browserNoteHbsState";
import { appStorageEventKey, getAppStorage } from "../../../runtime/appStorage";
import {flushStorage, storagePersistence} from "../../../runtime/storagePersistence";
import { createNoteDocumentFromText, noteDocumentPlainText, notePageId, decodeNoteDocument, inferReadingNoteType, noteBlockId, type NoteDocumentV1, type NoteLink, type NotePage, type ReadingNoteBlock, type NoteReadingSource } from "@research-assistant/core/notes";
import type { LiteratureAnnotation, LiteratureItem } from "@research-assistant/core/literature";
import { NotesGatewayError, type NoteIndexDocument, type NoteRevisionSummary } from "@research-assistant/data/notes";

const STORAGE_KEY = "research-assistant.notes.v1";
const LEGACY_ANNOTATION_KEY = "research-assistant.annotations.v1";
export const LITERATURE_KEY = "research-assistant.literature.v1";
export const NOTE_ASSET_PREFIX = "research-assistant.note-asset.v1.";
export const NOTE_TAG_KEY = "research-assistant.note-tags.v1";

export interface BrowserRevision extends NoteRevisionSummary {
  readonly document: NoteDocumentV1;
}

export interface BrowserNotesState {
  readonly persistence?: {readonly version: number; readonly before: StoredRows; readonly owner?: Storage};
  readonly metadataReceipts?: Readonly<Record<string, {readonly request: NoteMetadataBatchRequest | import("@research-assistant/core/notes").NoteMetadataReplayRequest; readonly receipt: NoteMetadataBatchReceipt}>>;
  readonly hbsRecords?: Readonly<Record<string, BrowserNoteHbsRecord>>;
  readonly favoriteOrder?: readonly NotePage["id"][];
  readonly conflictResolutions?: Readonly<Record<string, { readonly request: import("@research-assistant/data/notes").NoteConflictResolutionRequest; readonly receipt: import("@research-assistant/data/notes").NoteConflictResolutionReceipt }>>;
  readonly schemaVersion: 2;
  readonly pages: readonly NotePage[];
  readonly documents: Readonly<Record<string, NoteDocumentV1>>;
  readonly readingSources: readonly NoteReadingSource[];
  readonly links: readonly NoteLink[];
  readonly revisions: readonly BrowserRevision[];
}

export function readJsonArray<T>(key: string): readonly T[] {
  const raw = getAppStorage().getItem(key);
  if (raw === null) return [];
  try {
    const value: unknown = JSON.parse(raw);
    return Array.isArray(value) ? value as T[] : [];
  } catch {
    return [];
  }
}

function migrateLegacyAnnotations(): readonly ReadingNoteBlock[] {
  const literature = readJsonArray<LiteratureItem>(LITERATURE_KEY);
  return readJsonArray<LiteratureAnnotation>(LEGACY_ANNOTATION_KEY).map((annotation) => {
    const item = literature.find((candidate) => candidate.id === annotation.itemId);
    const attachment = item?.attachments.find((candidate) => candidate.id === annotation.attachmentId);
    return {
      id: noteBlockId(annotation.id),
      noteType: inferReadingNoteType(annotation.annotationType, annotation.quoteExact),
      body: annotation.body,
      color: annotation.color,
      status: "active",
      isFavorite: false,
      revision: 0,
      source: {
        anchorId: annotation.anchorId,
        paperId: annotation.itemId,
        paperTitle: item?.title ?? "来源文献",
        attachmentId: annotation.attachmentId,
        contentSha256: attachment?.contentSha256 ?? "",
        pageIndex: annotation.pageIndex,
        ...(annotation.pageLabel === undefined ? {} : { pageLabel: annotation.pageLabel }),
        rects: annotation.rects,
        quoteExact: annotation.quoteExact,
        quotePrefix: "",
        quoteSuffix: "",
        state: annotation.annotationType === "note" && annotation.quoteExact.length === 0 ? "page_only" : "exact",
      },
      projectIds: [],
      tagIds: [],
      createdAt: annotation.createdAt,
      updatedAt: annotation.createdAt,
    };
  });
}

function initialState(): BrowserNotesState {
  return migrateState({ schemaVersion: 1, pages: [], documents: {}, readingNotes: migrateLegacyAnnotations(), links: [], revisions: [] });
}

const DATABASE_TOKEN = STORAGE_KEY + ".indexeddb";
const databases = new WeakMap<Storage, {token: string; database: Promise<BrowserNotesDatabase>; conflict: boolean}>();
if (typeof window !== "undefined") window.addEventListener("storage", event => {
  if (storagePersistence(getAppStorage())) return;
  // A legacy tab writing the frozen import cannot overwrite the new database.
  if (event.key === appStorageEventKey(STORAGE_KEY)) {
    const storage = getAppStorage(), entry = databases.get(storage);
    if (entry) entry.conflict = true;
  }
});
function rowsOf(state: BrowserNotesState): StoredRows {
  return {pages: Object.fromEntries(state.pages.map(page => [page.id, page])), documents: {...state.documents},
    sources: Object.fromEntries(state.readingSources.map(source => [source.id, source])),
    links: Object.fromEntries(state.links.map(link => [link.id, link])),
    revisions: Object.fromEntries(state.revisions.map(revision => [revision.id, revision])),
    navigation: {favoriteOrder: state.favoriteOrder ?? []}, conflicts: {...state.conflictResolutions},
    operations: {...state.metadataReceipts, ...Object.fromEntries(Object.entries(state.hbsRecords ?? {}).map(([id, value]) => [browserHbsRecordKey(id), value]))}};
}
async function database(): Promise<BrowserNotesDatabase> {
  const storage = getAppStorage();
  let token = storage.getItem(DATABASE_TOKEN);
  if (!token) { token = crypto.randomUUID(); storage.setItem(DATABASE_TOKEN, token); }
  const existing = databases.get(storage);
  if (existing?.token === token) {
    if (existing.conflict) throw new NotesGatewayError("revision_conflict", "notes_legacy_snapshot_changed：旧标签页修改了迁移前的笔记。新旧数据均已保留，请关闭旧标签页并核对后重新打开。");
    return existing.database;
  }
  const pending = openNotesDatabase("research-assistant.notes." + token).then(async db => {
    // Preview migration is complete. The record database is authoritative; old
    // snapshots and import markers are not a runtime dependency of this path.
    if (storagePersistence(storage)) { await flushStorage(storage); return db; }
    const raw = storage.getItem(STORAGE_KEY);
    const fingerprint = raw === null ? null : Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(raw))), value => value.toString(16).padStart(2, "0")).join("");
    const initialized = await db.read({navigation: ["import"]});
    if (initialized.version !== 0) {
      const imported = initialized.rows.navigation?.import as {ready?: boolean; fingerprint?: string | null} | undefined;
      if (!imported?.ready)
        throw new NotesGatewayError("revision_conflict", `notes_migration_incomplete：笔记迁移完成标记${imported === undefined ? "缺失" : "未完成"}（databaseVersion=${initialized.version}；marker=${imported === undefined ? "missing" : "pending"}）；原始快照与已复制数据均已保留，请核对迁移状态后重试。`);
      if (imported.fingerprint !== fingerprint)
        throw new NotesGatewayError("revision_conflict", "notes_legacy_snapshot_changed：迁移前的原始快照与已记录指纹不一致；新旧数据均已保留，请关闭旧标签页并核对后重新打开。");
      return db;
    }
    let state: BrowserNotesState;
    try {
      const original = raw === null ? initialState() : JSON.parse(raw);
      const value = original.schemaVersion === 1 ? migrateState(original) : original as BrowserNotesState;
      if (value.schemaVersion !== 2 || !Array.isArray(value.pages) || !Array.isArray(value.revisions)) throw new Error("unsupported notes schema");
      state = withReadingOriginLinks({...value, documents: Object.fromEntries(Object.entries(value.documents).map(([id, doc]) => [id, decodeNoteDocument(doc)])),
        revisions: value.revisions.map(revision => ({...revision, isPinned: revision.isPinned ?? false}))});
    } catch (reason) { throw new NotesGatewayError("invalid_input", "浏览器笔记迁移失败；原始数据已保留。" + (reason instanceof Error ? reason.message : String(reason))); }
    const rows = rowsOf(state), changes: NoteWrite[] = [];
    for (const [store, values] of Object.entries(rows)) for (const [key, value] of Object.entries(values)) changes.push({store: store as NoteWrite["store"], key, value});
    changes.push({store: "navigation", key: "import", value: {ready: false, fingerprint}});
    try { await db.commit(0, changes); }
    catch (reason) {
      if (!(reason instanceof NotesGatewayError && reason.code === "revision_conflict")) throw reason;
      const concurrent = (await db.read({navigation: ["import"]})).rows.navigation?.import as {ready?: boolean; fingerprint?: string | null} | undefined;
      if (!concurrent?.ready || concurrent.fingerprint !== fingerprint) throw reason;
      return db;
    }
    const checked = await db.read(Object.fromEntries(Object.keys(rows).map(store => [store, "all"])) as NoteRead);
    for (const [store, values] of Object.entries(rows)) {
      const actual = {...checked.rows[store as NoteWrite["store"]]};
      if (store === "navigation") delete actual.import;
      const keys = Object.keys(values);
      if (Object.keys(actual).length !== keys.length || keys.some(key => JSON.stringify(actual[key]) !== JSON.stringify(values[key])))
        throw new NotesGatewayError("storage_error", `notes_migration_verification_failed：笔记迁移校验失败（store=${store}；expectedRows=${keys.length}；actualRows=${Object.keys(actual).length}；databaseVersion=${checked.version}）；原始数据已保留，未启用新存储。`);
    }
    if (storage.getItem(STORAGE_KEY) !== raw) throw new NotesGatewayError("revision_conflict", "迁移期间旧标签页修改了笔记，原始数据已保留。");
    await db.commit(checked.version, [{store: "navigation", key: "import", value: {ready: true, fingerprint}}]);
    // The old localStorage snapshot stays as an untouched migration backup.
    return db;
  });
  const entry = {token, database: pending, conflict: false}; databases.set(storage, entry);
  pending.catch(() => { if (databases.get(storage) === entry) databases.delete(storage); });
  return pending;
}
/** Search uses the same durable database owner and import guard as editing. */
export async function queryBrowserNoteSearch(query: import("@research-assistant/data/notes").NoteSearchQuery): Promise<import("@research-assistant/data/notes").NoteSearchPage> {
  const db = await database();
  if (!db.searchNotes) throw new NotesGatewayError("storage_error","note_search_adapter_unavailable：当前笔记存储未提供持久搜索查询。");
  return db.searchNotes(query);
}

/** Index loading needs neither full metadata scans nor revisions/conflict state. */
export async function readIndexDocuments(pageIds: readonly NotePage["id"][]): Promise<readonly NoteIndexDocument[]> {
  if (pageIds.length > 20) throw new NotesGatewayError("invalid_input", "notes_index_documents_limit：一次最多读取 20 篇笔记的索引正文。");
  const ids = [...new Set(pageIds)];
  if (!ids.length) return [];
  const {rows} = await (await database()).read({pages: ids, documents: ids});
  return ids.map(pageId => {
    const page = rows.pages?.[pageId] as NotePage | undefined;
    const document = rows.documents?.[pageId];
    if (!page || document === undefined) throw new NotesGatewayError("not_found", "找不到笔记页面");
    return {pageId, revision: page.revision, document: decodeNoteDocument(document)};
  });
}
/** Keeps catalog access behind the existing storage owner and migration guard. */
export async function queryBrowserPageCatalog(query: import("@research-assistant/core/notes").NoteIndexQuery, options?: {readonly signal?: AbortSignal}): Promise<import("@research-assistant/data/notes").NotePageCatalogPage> {
  options?.signal?.throwIfAborted();
  const db = await database();
  if (!db.queryCatalog) throw new NotesGatewayError("storage_error", "note_catalog_adapter_unavailable：当前笔记存储未提供持久目录查询。");
  options?.signal?.throwIfAborted();
  return db.queryCatalog(query,options);
}
export async function readState(options: {readonly metadataOnly?: boolean;readonly revisions?:boolean;
  readonly pageIds?: readonly string[] | ((metadata: BrowserNotesState) => readonly string[]); readonly operationId?: string; readonly hbsPageId?: string; readonly hbsPageIds?: readonly string[]} = {}): Promise<BrowserNotesState> {
  const owner = getAppStorage();
  const selectDocuments = typeof options.pageIds === "function" ? options.pageIds : undefined;
  const fixedPageIds = typeof options.pageIds === "function" ? undefined : options.pageIds;
  const query: NoteRead = {pages: "all", sources: "all", links: "all", navigation: ["favoriteOrder"],
    ...(!options.metadataOnly && !selectDocuments ? {documents: fixedPageIds ?? "all", ...(!options.pageIds||options.revisions ? {revisions: "all"} : {}),...(!options.pageIds?{conflicts:"all"}:{})} : {}),
    ...(options.operationId || options.hbsPageId || options.hbsPageIds?.length ? {operations: [...(options.operationId ? [options.operationId] : []), ...(options.hbsPageId ? [browserHbsRecordKey(options.hbsPageId)] : []), ...(options.hbsPageIds?.map(browserHbsRecordKey) ?? [])]} : {})};
  // An explicit history read also applies to a bounded document selector;
  // otherwise lifecycle inverses lose their original entrance identity.
  if(options.revisions&&!options.metadataOnly)query.revisions="all";
  const {version, rows} = await (await database()).read(query,
    !options.metadataOnly && selectDocuments ? (rows, version) => selectDocuments(stateFromRows(version, rows, owner)) : undefined);
  return stateFromRows(version, rows, owner);
}
/** Authority reads and S′ updates need only this document's S/B/checkpoint.
 * In particular they must not synthesize unrelated reading-origin links. The
 * partial before-image still participates in the same database-wide CAS. */
export async function readNoteHbsState(id: string): Promise<BrowserNotesState> {
  const owner=getAppStorage();
  const {version,rows}=await (await database()).read({pages:[id],documents:[id],operations:[browserHbsRecordKey(id)]});
  return stateFromRows(version,rows,owner);
}
/** Metadata and direct outgoing relations for opened identities only. */
export async function readNotePageState(ids:readonly string[],documents=false):Promise<BrowserNotesState>{
  const owner=getAppStorage();
  const {version,rows}=await(await database()).read({pages:ids,sources:{index:"pageId",keys:ids},links:{index:"sourcePageId",keys:ids},...(documents?{documents:ids}:{})});
  return stateFromRows(version,rows,owner);
}
/** Exact metadata existence for host association; never opens note bodies. */
export async function browserNoteHostExists(kind: "note" | "readingNote", id: string): Promise<boolean> {
  const db = await database();
  if (kind === "readingNote") return (await db.read({sources: [id]})).rows.sources?.[id] !== undefined;
  const {rows} = await db.read({pages: [id], sources: {index: "pageId", keys: [id]}});
  return rows.pages?.[id] !== undefined && Object.keys(rows.sources ?? {}).length === 0;
}
/** A reading alias is resolved by source primary key or its page index. The
 * second bounded read must have the same canonical version; never mix sources
 * from an old transaction with a new document. */
export async function readReadingNoteState(id:string):Promise<BrowserNotesState>{
  const db=await database();
  for(let attempt=0;attempt<3;attempt++){
    const first=await db.read({sources:{index:"pageId",keys:[id],primaryKeys:[id]}});
    const source=Object.values(first.rows.sources??{})[0] as NoteReadingSource|undefined;
    if(!source)throw new NotesGatewayError("not_found","找不到阅读笔记");
    const state=await readNotePageState([source.pageId],true);
    if(state.persistence!.version===first.version)return state;
  }
  throw new NotesGatewayError("revision_conflict","note_reading_changed：阅读笔记在读取期间更新，请重试。");
}
/** Opened-document relations read link metadata and their exact targets. The
 * legacy backlink scan visits links only, never every note page/source/body. */
export async function readNoteRelations(query:{readonly pageId:string}|{readonly targetKind:NoteLink["target"]["kind"];readonly targetId:string}):Promise<BrowserNotesState>{
  const owner=getAppStorage(),db=await database();
  for(let attempt=0;attempt<3;attempt++){
    const pageId="pageId"in query?query.pageId:undefined;
    const first=await db.read(pageId?{pages:[pageId],sources:{index:"pageId",keys:[pageId]},links:{index:"sourcePageId",keys:[pageId]}}:{links:"all"});
    const state=stateFromRows(first.version,first.rows,owner);
    const links="targetId"in query?state.links.filter(link=>link.target.kind===query.targetKind&&link.target.id===query.targetId):state.links;
    const pageIds=[...new Set(links.flatMap(link=>[
      ...(link.target.kind==="note_page"?[link.target.id]:[]),
      ...("targetId"in query?[link.sourcePageId]:[]),
    ]))];
    const sourceIds=[...new Set(links.flatMap(link=>link.target.kind==="reading_note"?[link.target.id]:[]))];
    if(!pageIds.length&&!sourceIds.length)return{...state,links};
    const targets=await db.read({pages:pageIds,sources:sourceIds});
    if(targets.version!==first.version)continue;
    return{...state,links,pages:[...state.pages,...Object.values(targets.rows.pages??{})as NotePage[]],readingSources:[...state.readingSources,...Object.values(targets.rows.sources??{})as NoteReadingSource[]]};
  }
  throw new NotesGatewayError("revision_conflict","note_relations_changed：笔记关联在读取期间更新，请重试。");
}
export async function writeNoteHbsRecord(state: BrowserNotesState,id:string,value:BrowserNoteHbsRecord):Promise<void>{
  if(state.persistence?.owner!==getAppStorage())throw new NotesGatewayError("operation_cancelled","笔记存储拥有者已切换，旧检查点未写入。");
  await (await database()).commit(state.persistence.version,[{store:"operations",key:browserHbsRecordKey(id),value}]);
}
/** Backlink reads may remove orphan relation rows only. Their partial read
 * must never be written as a complete note snapshot or erase unrelated links. */
export async function removeMissingSourceLinks(state: BrowserNotesState, ids: readonly string[]): Promise<void> {
  if (!ids.length) return;
  if (state.persistence?.owner !== getAppStorage()) throw new NotesGatewayError("operation_cancelled", "笔记存储拥有者已切换，反链清理未执行。");
  await (await database()).commit(state.persistence.version, ids.map(key => ({store: "links", key})));
}
function stateFromRows(version: number, rows: StoredRows, owner: Storage): BrowserNotesState {
  const documents = Object.fromEntries(Object.entries(rows.documents ?? {}).map(([id, doc]) => [id, decodeNoteDocument(doc)]));
  if (rows.documents) rows.documents = documents;
  return withReadingOriginLinks({schemaVersion: 2, pages: Object.values(rows.pages ?? {}) as NotePage[], documents,
    readingSources: Object.values(rows.sources ?? {}) as NoteReadingSource[], links: Object.values(rows.links ?? {}) as NoteLink[],
    revisions: Object.values(rows.revisions ?? {}) as BrowserRevision[], favoriteOrder: (rows.navigation?.favoriteOrder ?? []) as NotePage["id"][],
    ...(rows.conflicts ? {conflictResolutions: rows.conflicts as NonNullable<BrowserNotesState["conflictResolutions"]>} : {}),
    ...(rows.operations ? {
      metadataReceipts: Object.fromEntries(Object.entries(rows.operations).filter(([id]) => !id.startsWith("$hbs:"))) as NonNullable<BrowserNotesState["metadataReceipts"]>,
      hbsRecords: Object.fromEntries(Object.entries(rows.operations).filter(([id]) => id.startsWith("$hbs:")).map(([id, value]) => [id.slice(5), value])) as NonNullable<BrowserNotesState["hbsRecords"]>,
    } : {}),
    persistence: {version, before: rows, owner}});
}
export async function writeState(value: BrowserNotesState): Promise<void> {
  const owner = getAppStorage();
  if (value.persistence?.owner && value.persistence.owner !== owner)
    throw new NotesGatewayError("operation_cancelled", "笔记存储拥有者已切换，旧快照未写入。");
  const db = await database(), state = withReadingOriginLinks(value);
  const base = state.persistence ?? (await readState()).persistence!;
  const next = rowsOf(state), changes: NoteWrite[] = [];
  for (const [store, values] of Object.entries(next)) {
    const before = base.before[store as NoteWrite["store"]] ?? {};
    for (const key of new Set([...Object.keys(before), ...Object.keys(values)])) {
      if (before[key] === values[key]) continue;
      if (before[key] !== undefined && values[key] !== undefined && JSON.stringify(before[key]) === JSON.stringify(values[key])) continue;
      changes.push({store: store as NoteWrite["store"], key, ...(values[key] === undefined ? {} : {value: values[key]})});
    }
  }
  if (getAppStorage() !== owner) throw new NotesGatewayError("operation_cancelled", "笔记存储拥有者已切换，操作未执行。");
  await db.commit(base.version, changes);
}

/** The same page-to-paper link is queried in both directions. No duplicate body. */
function withReadingOriginLinks(state: BrowserNotesState): BrowserNotesState {
  const links = [...state.links];
  const origins = new Set(links.filter(link => link.sourceBlockId === undefined && link.target.kind === "paper" && link.relation === "derived_from")
    .map(link => JSON.stringify([link.sourcePageId, link.target.id])));
  const pages = new Map(state.pages.map(page => [page.id, page]));
  for (const record of state.readingSources) {
    const identity = JSON.stringify([record.pageId, record.source.paperId]);
    if (origins.has(identity)) continue;
    origins.add(identity);
    links.push({ id: `reading-origin:${record.id}` as NoteLink["id"], sourcePageId: record.pageId,
      target: { kind: "paper", id: record.source.paperId, labelSnapshot: record.source.paperTitle, quoteSnapshot: record.source.quoteExact },
      relation: "derived_from", isBroken: false, createdAt: pages.get(record.pageId)?.createdAt ?? "" });
  }
  const sources = new Map(state.readingSources.map(record => [record.pageId, record]));
  const favorites = state.pages.filter(page => page.isFavorite && !((source => source && (source.noteType === "highlight" || source.noteType === "annotation" && source.source.quoteExact.trim() !== ""))(sources.get(page.id))))
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt) || a.id.localeCompare(b.id)).map(page => page.id);
  const existing = (state.favoriteOrder ?? []).filter(id => favorites.includes(id));
  return { ...state, links, favoriteOrder: [...favorites.filter(id => !existing.includes(id)), ...existing] };
}

export function now(): string {
  return new Date().toISOString();
}

interface LegacyNotesState extends Omit<BrowserNotesState, "schemaVersion" | "readingSources"> {
  readonly schemaVersion: 1; readonly readingNotes: readonly ReadingNoteBlock[];
}
function migrateState(legacy: LegacyNotesState): BrowserNotesState {
  const pages = [...legacy.pages], documents = { ...legacy.documents };
  const readingSources = legacy.readingNotes.map(note => {
    const pageId = notePageId(crypto.randomUUID());
    const { id, body, source, noteType, color, pageId: _oldPageId, ...properties } = note;
    pages.push({ ...properties, id: pageId, title: (source.paperTitle || "阅读笔记").slice(0, 240), position: pages.length, schemaVersion: 1 });
    documents[pageId] = createNoteDocumentFromText(pageId, body, () => noteBlockId(crypto.randomUUID()));
    return { id, pageId, noteType, color, source };
  });
  return { schemaVersion: 2, pages, documents, readingSources, links: legacy.links, revisions: legacy.revisions };
}

/** Read projection only: there is no persisted annotation body or lifecycle copy. */
export function readingNotesFromState(state: BrowserNotesState): readonly ReadingNoteBlock[] {
  const pages = new Map(state.pages.map(page => [page.id, page]));
  return state.readingSources.flatMap(record => {
    const page = pages.get(record.pageId);
    const document = state.documents[record.pageId];
    if (!page || !document) return [];
    return [{ ...page, ...record, body: noteDocumentPlainText(document) }];
  });
}
