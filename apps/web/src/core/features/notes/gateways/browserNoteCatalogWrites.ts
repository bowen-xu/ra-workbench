import {decodeNoteDocument, noteDocumentPlainText, type NotePage, type NoteReadingSource, type NoteLink} from "@research-assistant/core/notes";
import {projectNoteCatalogHeader, noteSearchGrams, NOTE_SEARCH_GRAM_VERSION, type NoteCatalogHeader, type NoteCatalogRuntime, NotesGatewayError} from "@research-assistant/data/notes";
import {catalogRequest, type BrowserCatalogBody, type BrowserCatalogState} from "./browserNoteCatalogStore";
import type {NoteWrite} from "./browserNotesDatabase";
import {catalogPostingUpdater} from "./browserCatalogPostings";

import {invalidateCatalogScope} from "./browserNoteCatalogScopes";

/** Keep no-op display projections off the multiEntry sorting index. Compare
 * text metadata independently of binary keys; authoring revisions/generation
 * are owned by the caller and must still advance for an actual body save. */
function sameHeader(before: NoteCatalogHeader | undefined, after: NoteCatalogHeader): boolean {
  if (!before) return false;
  const {orderKeys: oldKeys,...oldMetadata} = before;
  const {orderKeys: newKeys,...newMetadata} = after;
  return JSON.stringify(oldMetadata) === JSON.stringify(newMetadata)
    && oldKeys.length === newKeys.length && oldKeys.every(([bucket,key,id],index) => {
      const next = newKeys[index]!;
      return bucket === next[0] && id === next[2] && key.length === next[1].length
        && key.every((byte,offset) => byte === next[1][offset]);
    });
}

/** Canonical saves call this after CAS; backfill supplies current transaction
 * reads without changing canonical data. Every request uses that transaction.
 * Initialize the runtime first; no external I/O may occur between requests.
 * knownMissingBodies may only contain keys checked absent in this transaction;
 * it must never survive a transaction or replace ordinary-save reads. */
export async function applyBrowserNoteCatalogChanges(transaction: IDBTransaction, changes: readonly NoteWrite[], runtime: NoteCatalogRuntime,
  options: {readonly favoritePositions?: ReadonlyMap<string,number>;readonly knownMissingBodies?: ReadonlySet<string>} = {}): Promise<void> {
  const states = transaction.objectStore("catalogState");
  const current = await catalogRequest(states.get("current")) as BrowserCatalogState | undefined;
  let state: BrowserCatalogState = current ?? {generation: 0,nextOrdinal: 1,collationVersion: runtime.collationVersion,normalizationVersion: runtime.normalizationVersion,gramVersion: NOTE_SEARCH_GRAM_VERSION,searchReady: false};
  if (state.gramVersion !== NOTE_SEARCH_GRAM_VERSION || state.collationVersion !== runtime.collationVersion || state.normalizationVersion !== runtime.normalizationVersion)
    throw new NotesGatewayError("index_failed", "note_catalog_projection_version：笔记查询投影版本已变化，请重建派生索引，原始笔记未改动。");
  const headers = transaction.objectStore("catalogHeaders"), bodies = transaction.objectStore("catalogBodies");
  const affected = new Set<string>();
  const sourceChanges = new Map<string, NoteWrite[]>(), linkChanges = new Map<string, NoteWrite[]>();
  const incoming = new Map(changes.map(change => [`${change.store}:${change.key}`, change]));
  const valueAfter = async <T>(store: "pages" | "sources" | "links" | "navigation", key: string): Promise<T | undefined> => {
    const found = incoming.get(`${store}:${key}`);
    return found ? found.value as T | undefined : catalogRequest(transaction.objectStore(store).get(key)) as Promise<T | undefined>;
  };
  for (const change of changes) {
    if (change.store === "pages" || change.store === "documents") affected.add(change.key);
    if (change.store === "sources" || change.store === "links") {
      const old = await catalogRequest(transaction.objectStore(change.store).get(change.key)) as NoteReadingSource | NoteLink | undefined;
      const pageId = (value: NoteReadingSource | NoteLink | undefined) => value && ("pageId" in value ? value.pageId : value.sourcePageId);
      for (const id of new Set([pageId(old), pageId(change.value as typeof old)])) if (id) {
        affected.add(id);
        const grouped = change.store === "sources" ? sourceChanges : linkChanges;
        grouped.set(id, [...grouped.get(id) ?? [],change]);
      }
    }
    if (change.store === "navigation" && change.key === "favoriteOrder") {
      const before = await catalogRequest(transaction.objectStore("navigation").get("favoriteOrder")) as string[] | undefined;
      for (const id of [...before ?? [],...change.value as string[] ?? []]) affected.add(id);
    }
  }
  const updateGrams = catalogPostingUpdater(transaction);
  const favoriteChange = incoming.get("navigation:favoriteOrder");
  const positions = favoriteChange ? new Map(((favoriteChange.value ?? []) as readonly string[]).map((id, index) => [id, index])) : options.favoritePositions;
  for (const pageId of affected) {
    const itemId = `page:${pageId}`;
    const previous = await catalogRequest(headers.get(itemId)) as NoteCatalogHeader | undefined;
    const page = await valueAfter<NotePage>("pages", pageId);
    if (!page) {
      if (previous) {
        const body = await catalogRequest(bodies.get(itemId)) as BrowserCatalogBody | undefined;
        await updateGrams(previous.ordinal * 2, new Set(body?.grams), new Set());
        await updateGrams(previous.ordinal * 2 + 1, noteSearchGrams(previous.metadataText), new Set());
        headers.delete(itemId); bodies.delete(itemId);
      }
      continue;
    }
    const sourceRows = await catalogRequest(transaction.objectStore("sources").index("pageId").getAll(pageId)) as NoteReadingSource[];
    const sourceMap = new Map(sourceRows.map(source => [String(source.id), source]));
    const linkRows = await catalogRequest(transaction.objectStore("links").index("sourcePageId").getAll(pageId)) as NoteLink[];
    const linkMap = new Map(linkRows.map(link => [String(link.id), link]));
    for (const change of [...sourceChanges.get(pageId) ?? [],...linkChanges.get(pageId) ?? []]) {
      if (change.store === "sources") {
        const source = change.value as NoteReadingSource | undefined;
        sourceMap.delete(change.key); if (source?.pageId === pageId) sourceMap.set(change.key, source);
      } else {
        const link = change.value as NoteLink | undefined;
        linkMap.delete(change.key); if (link?.sourcePageId === pageId) linkMap.set(change.key, link);
      }
    }
    if (sourceMap.size > 1) throw new NotesGatewayError("index_failed", "note_catalog_duplicate_source：同一笔记页面有多个阅读来源，请核对原始数据。");
    const ordinal = previous?.ordinal ?? state.nextOrdinal;
    if (!previous) state = {...state,nextOrdinal: ordinal + 1};
    const document = incoming.get(`documents:${pageId}`);
    const source = sourceMap.values().next().value;
    // Ordinary titles come from page metadata. Only reading titles need a
    // first-line cache in this indexed header. A later source attachment uses
    // the current persisted body projection, never a stale ordinary cache.
    let bodyFirstLine = source ? previous?.bodyFirstLine ?? "" : "";
    if (source && previous?.item.target.kind !== "reading" && document?.value === undefined) {
      const indexedBody = await catalogRequest(bodies.get(itemId)) as BrowserCatalogBody | undefined;
      if (indexedBody) bodyFirstLine = runtime.displayTitle(indexedBody.text,"","");
      else {
        state = invalidateCatalogScope(state,page.status);
      }
    }
    if (document?.value === undefined && (!previous || previous.item.status !== page.status)) {
      // Status moves can introduce an unindexed page before a committed range
      // cursor. A key-only proof keeps metadata-only saves free of body values.
      if (!previous || !await catalogRequest(bodies.getKey(itemId))) state = invalidateCatalogScope(state,page.status);
    }
    if (document) {
      const before = options.knownMissingBodies?.has(pageId) ? undefined
        : await catalogRequest(bodies.get(itemId)) as BrowserCatalogBody | undefined;
      if (document.value === undefined) {
        await updateGrams(ordinal * 2, new Set(before?.grams), new Set()); bodies.delete(itemId); bodyFirstLine = "";
        state = invalidateCatalogScope(state,page.status);
      } else {
        const text = noteDocumentPlainText(decodeNoteDocument(document.value));
        const foldedText = runtime.fold(text), grams = noteSearchGrams(foldedText);
        bodyFirstLine = source ? runtime.displayTitle(text, "", "") : "";
        await updateGrams(ordinal * 2, new Set(before?.grams), grams);
        bodies.put({text,foldedText,grams: [...grams]} satisfies BrowserCatalogBody, itemId);
      }
    }
    const favoritePosition = positions ? positions.get(pageId) : previous?.item.favoritePosition;
    const header = projectNoteCatalogHeader({page,...(source ? {source} : {}),links: [...linkMap.values()],bodyFirstLine,ordinal,
      ...(favoritePosition === undefined ? {} : {favoritePosition})}, runtime);
    await updateGrams(ordinal * 2 + 1, noteSearchGrams(previous?.metadataText ?? ""), noteSearchGrams(header.metadataText));
    if (!sameHeader(previous,header)) headers.put(header);
  }
  updateGrams.flush();
  if (affected.size || !current) states.put({...state,generation: state.generation + 1}, "current");
}
