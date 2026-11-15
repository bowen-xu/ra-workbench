import { browserNoteReads } from "./browserNoteReads";
import {reconcileBrowserPageEntries,type PageEntryLifecycleMode} from "./browserPageEntryLifecycle";
import {browserNotePageCatalogGateway} from "./browserNotePageCatalogGateway";
import { searchBrowserNotes } from "./browserNoteSearch";
import { browserNoteNavigation } from "./browserNoteNavigation";
import { browserNoteMetadataBatch } from "./browserNoteMetadataBatch";
import { serializeBrowserNotesWrites, withBrowserNotesWrite } from "./browserNotesWrites";
import { pruneBrowserRevisions, resolveBrowserConflict } from "./browserNoteDocument";
import { getAppStorage } from "../../../runtime/appStorage";
import { flushStorage } from "../../../runtime/storagePersistence";
import {
  createEmptyNoteDocument,
  createNoteDocumentFromText,
  decodeNoteDocument,
  noteBlockId,
  noteLinkId,
  notePageId,
  noteRevisionId,
  normalizeNoteTitle,
  noteImageDisplayName,
  isPositionReadingNote,
  readingNoteSourceAt,
  isDependentReadingNote,
  type NoteDocumentV1,
  type NoteLink,
  type NotePage,
  type NotePageId,
  type ReadingNoteBlock,
} from "@research-assistant/core/notes";
import type { LiteratureItem } from "@research-assistant/core/literature";
import {browserNoteSourceLabels} from "./browserNoteSourceLabels";
import {
  NotesGatewayError,
  type NoteDocumentSaveRequest,
  type NoteDocumentSnapshot,
  type NoteLinkDraft,
  type NoteListQuery,
  type NotePageDraft,
  type NotePageMetadataUpdate,
  type NotePageMoveRequest,
  type NotesGateway,
  type ReadingNoteDraft,
  type ReadingNoteUpdate,
} from "@research-assistant/data/notes";
import {
  LITERATURE_KEY,
  NOTE_ASSET_PREFIX,
  NOTE_TAG_KEY,
  now,
  readJsonArray,
  readState,
  readNotePageState,
  readNoteRelations,
  removeMissingSourceLinks,
  readIndexDocuments,
  readingNotesFromState,
  writeState,
  type BrowserNotesState,
  type BrowserRevision,
} from "./browserNotesStorage";

function presentPage(state: BrowserNotesState, page: NotePage): NotePage {
  const { literaturePaperId: _previous, ...base } = page;
  const source = state.links.find(link => link.sourcePageId === page.id && link.sourceBlockId === undefined
    && link.relation === "derived_from" && link.target.kind === "paper");
  return source ? { ...base, literaturePaperId: source.target.id } : base;
}

function snapshot(state: BrowserNotesState, pageId: NotePageId): NoteDocumentSnapshot {
  const page = state.pages.find((candidate) => candidate.id === pageId);
  const document = state.documents[pageId];
  if (page === undefined || document === undefined) throw new NotesGatewayError("not_found", "找不到笔记页面");
  return { page: presentPage(state, page), document: {...document, title: document.title ?? page.title} };
}


function replacePage(state: BrowserNotesState, page: NotePage): BrowserNotesState {
  return { ...state, pages: state.pages.map((candidate) => candidate.id === page.id ? page : candidate) };
}

function touchBrowserParents(state: BrowserNotesState, parentIds: ReadonlySet<NotePageId>): BrowserNotesState {
  const timestamp = now();
  return {
    ...state,
    pages: state.pages.map((page) => parentIds.has(page.id)
      ? { ...page, revision: page.revision + 1, updatedAt: timestamp }
      : page),
  };
}

function synchronizeChildPages(state: BrowserNotesState): BrowserNotesState {
  const documents = Object.fromEntries(Object.entries(state.documents).map(([pageId, document]) => {
    if(state.pages.find(page=>page.id===pageId)?.status!=="active")return[pageId,document];
    const children = state.pages
      .filter((page) => page.parentPageId === pageId && page.status === "active")
      .sort((left, right) => left.position - right.position || left.id.localeCompare(right.id));
    const childBlocks: NoteDocumentV1["blocks"] = children.map((child) => ({
      id: noteBlockId(`child-page:${child.id}`),
      type: "childPage",
      props: {},
      content: {
        kind: "reference",
        targetKind: "note_page",
        targetId: child.id,
        labelSnapshot: child.title,
        quoteSnapshot: "",
      },
      children: [],
    }));
    let childIndex = 0;
    const blocks = document.blocks.flatMap((block) => {
      if (block.type !== "childPage") return [block];
      const child = childBlocks[childIndex++];
      return child === undefined ? [] : [child];
    });
    blocks.push(...childBlocks.slice(childIndex));
    return [pageId, { ...document, blocks }];
  })) as Readonly<Record<string, NoteDocumentV1>>;
  let next: BrowserNotesState = { ...state, documents };
  for (const document of Object.values(documents)) {
    next = { ...next, links: semanticLinks(next, document) };
  }
  return next;
}

function descendantIds(pages: readonly NotePage[], rootId: NotePageId): ReadonlySet<NotePageId> {
  const ids = new Set<NotePageId>([rootId]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const page of pages) if (page.parentPageId !== undefined && ids.has(page.parentPageId) && !ids.has(page.id)) {
      ids.add(page.id);
      changed = true;
    }
  }
  return ids;
}

function childTargets(document: NoteDocumentV1): ReadonlySet<NotePageId> {
  const targets = new Set<NotePageId>();
  for (const block of document.blocks) if (block.type === "childPage" && block.content.kind === "reference") {
    if (block.content.targetKind !== "note_page" || targets.has(notePageId(block.content.targetId))) {
      throw new NotesGatewayError("invalid_input", "子页面块目标无效或重复");
    }
    targets.add(notePageId(block.content.targetId));
  }
  return targets;
}

function assertRevision(actual: number, expected: number): void {
  if (actual !== expected) throw new NotesGatewayError("revision_conflict", `笔记已更新，当前版本为 ${actual}`, actual);
}

function remapDocument(document: NoteDocumentV1, pageId: NotePageId): NoteDocumentV1 {
  const remap = (block: NoteDocumentV1["blocks"][number]): NoteDocumentV1["blocks"][number] => ({
    ...block, id: noteBlockId(crypto.randomUUID()), children: block.children.map(remap),
  });
  return { ...document, pageId, blocks: document.blocks.map(remap) };
}

function semanticLinks(state: BrowserNotesState, document: NoteDocumentV1): readonly NoteLink[] {
  const retained = state.links.filter((link) => link.sourcePageId !== document.pageId || link.relation === "derived_from" || link.sourceBlockId === undefined);
  const generated: NoteLink[] = [];
  const add = (blockId: NoteDocumentV1["blocks"][number]["id"], target: NoteLink["target"], relation: NoteLink["relation"]): void => {
    const existing = state.links.find((link) => link.sourcePageId === document.pageId && link.sourceBlockId === blockId && link.target.kind === target.kind && link.target.id === target.id && link.relation === relation);
    generated.push(existing ?? { id: noteLinkId(crypto.randomUUID()), sourcePageId: document.pageId, sourceBlockId: blockId, target, relation, isBroken: false, createdAt: now() });
  };
  const scanInline = (value: unknown, blockId: NoteDocumentV1["blocks"][number]["id"]): void => {
    if (Array.isArray(value)) { value.forEach((item) => scanInline(item, blockId)); return; }
    if (typeof value !== "object" || value === null) return;
    const item = value as Record<string, unknown>;
    if (item.type === "mention" && typeof item.targetKind === "string" && typeof item.targetId === "string") {
      add(blockId, { kind: item.targetKind as NoteLink["target"]["kind"], id: item.targetId, labelSnapshot: typeof item.labelSnapshot === "string" ? item.labelSnapshot : "研究对象" }, "mention");
    }
    Object.values(item).forEach((child) => scanInline(child, blockId));
  };
  const visit = (blocks: NoteDocumentV1["blocks"]): void => {
    for (const block of blocks) {
      if (block.content.kind === "reference") add(block.id, { kind: block.content.targetKind, id: block.content.targetId, labelSnapshot: block.content.labelSnapshot, quoteSnapshot: block.content.quoteSnapshot }, "reference");
      scanInline(block.content, block.id);
      visit(block.children);
    }
  };
  visit(document.blocks);
  return [...retained, ...generated];
}

function targetExists(state: BrowserNotesState, link: NoteLink): boolean {
  if (link.target.kind === "note_page") return state.pages.some((page) => page.id === link.target.id);
  if (link.target.kind === "reading_note") return state.readingSources.some((note) => note.id === link.target.id);
  if (link.target.kind === "paper") return readJsonArray<LiteratureItem>(LITERATURE_KEY).some((paper) => paper.id === link.target.id);
  return true;
}

function withBrokenState(state: BrowserNotesState, links: readonly NoteLink[]): readonly NoteLink[] {
  return links.map((link) => ({ ...link, isBroken: !targetExists(state, link) }));
}

function reparentAndOrder(
  pages: readonly NotePage[],
  moving: NotePage,
  parentPageId: NotePageId | undefined,
  requestedPosition: number,
): readonly NotePage[] {
  const timestamp = now();
  const oldParent = moving.parentPageId;
  const oldSiblings = pages.filter((page) => page.id !== moving.id && page.parentPageId === oldParent)
    .sort((left, right) => left.position - right.position || left.id.localeCompare(right.id));
  const newSiblings = oldParent === parentPageId
    ? oldSiblings
    : pages.filter((page) => page.id !== moving.id && page.parentPageId === parentPageId)
      .sort((left, right) => left.position - right.position || left.id.localeCompare(right.id));
  const position = Math.min(Math.max(0, requestedPosition), newSiblings.length);
  const orderedTarget = [...newSiblings.slice(0, position), moving, ...newSiblings.slice(position)];
  const positions = new Map<NotePageId, { readonly parentPageId: NotePageId | undefined; readonly position: number }>();
  if (oldParent !== parentPageId) oldSiblings.forEach((page, index) => positions.set(page.id, { parentPageId: oldParent, position: index }));
  orderedTarget.forEach((page, index) => positions.set(page.id, { parentPageId, position: index }));
  return pages.map((page) => {
    const next = positions.get(page.id);
    if (next === undefined) return page;
    const changed = page.id === moving.id || page.position !== next.position || page.parentPageId !== next.parentPageId;
    if (!changed) return page;
    const { parentPageId: _parentPageId, ...withoutParent } = page;
    return {
      ...withoutParent,
      ...(next.parentPageId === undefined ? {} : { parentPageId: next.parentPageId }),
      position: next.position,
      revision: page.revision + 1,
      updatedAt: timestamp,
    };
  });
}

async function createBrowserPage(draft: NotePageDraft, paperId?: string, assignedId?: NotePageId): Promise<NoteDocumentSnapshot> {
    const state = await readState();
    if (assignedId && state.pages.some(page => page.id === assignedId)) return snapshot(state, assignedId);
    const paper = paperId === undefined ? undefined : readJsonArray<LiteratureItem>(LITERATURE_KEY).find(item => item.id === paperId && item.trashedAt === undefined);
    if (paperId !== undefined && paper === undefined) throw new NotesGatewayError("not_found", "找不到文献项");
    if (draft.parentPageId === undefined && draft.parentBlockPosition !== undefined) {
      throw new NotesGatewayError("invalid_input", "顶层页面不能指定父页面块位置");
    }
    if (draft.parentBlockPosition !== undefined && (!Number.isSafeInteger(draft.parentBlockPosition) || draft.parentBlockPosition < 0)) {
      throw new NotesGatewayError("invalid_input", "子页面块位置无效");
    }
    if (draft.parentPageId !== undefined && !state.pages.some((page) => page.id === draft.parentPageId && page.status !== "trashed")) {
      throw new NotesGatewayError("not_found", "找不到父页面");
    }
    const id = assignedId ?? notePageId(crypto.randomUUID());
    const timestamp = now();
    const parentDocument = draft.parentPageId === undefined ? undefined : state.documents[draft.parentPageId];
    const requestedBlockPosition = draft.parentBlockPosition === undefined
      ? parentDocument?.blocks.length
      : Math.min(Math.max(0, draft.parentBlockPosition), parentDocument?.blocks.length ?? 0);
    const siblingPosition = parentDocument === undefined || requestedBlockPosition === undefined
      ? state.pages.filter((candidate) => candidate.parentPageId === draft.parentPageId).length
      : parentDocument.blocks.slice(0, requestedBlockPosition).filter((block) => block.type === "childPage").length;
    const page: NotePage = {
      id, title: normalizeNoteTitle(draft.title),
      ...(draft.parentPageId === undefined ? {} : { parentPageId: draft.parentPageId }),
      position: siblingPosition,
      status: "active", isFavorite: false, revision: 0, schemaVersion: 1,
      projectIds: [], tagIds: [], createdAt: timestamp, updatedAt: timestamp,
    };
    const document = createEmptyNoteDocument(id, noteBlockId(crypto.randomUUID()));
    const pages = state.pages.map((candidate) => candidate.parentPageId === draft.parentPageId && candidate.position >= siblingPosition
      ? { ...candidate, position: candidate.position + 1, revision: candidate.revision + 1, updatedAt: timestamp }
      : candidate);
    let documents = { ...state.documents, [id]: document };
    if (draft.parentPageId !== undefined && parentDocument !== undefined && requestedBlockPosition !== undefined) {
      const blocks = [...parentDocument.blocks];
      blocks.splice(requestedBlockPosition, 0, {
        id: noteBlockId(`child-page:${id}`),
        type: "childPage",
        props: {},
        content: { kind: "reference", targetKind: "note_page", targetId: id, labelSnapshot: page.title, quoteSnapshot: "" },
        children: [],
      });
      documents = { ...documents, [draft.parentPageId]: { ...parentDocument, blocks } };
    }
    const links = paper === undefined ? state.links : [...state.links, { id: noteLinkId(crypto.randomUUID()), sourcePageId: id, target: { kind: "paper" as const, id: paper.id, labelSnapshot: paper.title }, relation: "derived_from" as const, isBroken: false, createdAt: timestamp }];
    let next: BrowserNotesState = { ...state, pages: [...pages, page], documents, links };
    if (draft.parentPageId !== undefined) next = touchBrowserParents(next, new Set([draft.parentPageId]));
    next = synchronizeChildPages(next);
    await writeState(next);
    return { page, document };
}

/** Stable creation intent lets NAL retry a failed link without creating another page. */
export async function ensureBrowserAnnotationPage(target: readonly [string,string], title: string): Promise<NoteDocumentSnapshot> {
  const digest=await crypto.subtle.digest("SHA-256",new TextEncoder().encode(JSON.stringify(target)));
  const key=Array.from(new Uint8Array(digest),byte=>byte.toString(16).padStart(2,"0")).join("");
  return withBrowserNotesWrite(() => createBrowserPage({title},undefined,notePageId(`nal-note:${key}`)));
}

export function saveBrowserDocument(state: BrowserNotesState, request: NoteDocumentSaveRequest,lifecycle:PageEntryLifecycleMode="document"): BrowserNotesState {
    const current = snapshot(state, request.document.pageId); assertRevision(current.page.revision, request.expectedRevision);
    const document = decodeNoteDocument({...request.document, title: request.document.title ?? current.page.title});
    if(lifecycle==="document")state = reconcileBrowserPageEntries(state,current.document,document,now());
    const requestedChildren = childTargets(document);
    const requestedOrder = [...requestedChildren];
    const directChildren = new Set(state.pages.filter((page) => page.parentPageId === current.page.id && page.status === "active").map((page) => page.id));
    if ([...requestedChildren].some((id) => !directChildren.has(id))) throw new NotesGatewayError("invalid_input", "子页面块必须指向当前页面的活动子页面");
    const childPositions = new Map(requestedOrder.map((id, index) => [id, index]));
    state = { ...state, pages: state.pages.map((candidate) => {
      const position = childPositions.get(candidate.id);
      if (position === undefined || position === candidate.position) return candidate;
      return { ...candidate, position, revision: candidate.revision + 1, updatedAt: now() };
    }) };
    const revision: BrowserRevision = { id: noteRevisionId(crypto.randomUUID()), pageId: current.page.id, pageRevision: current.page.revision, reason: request.saveReason, createdAt: now(), isPinned: false, document: current.document };
    const page: NotePage = { ...current.page, title: normalizeNoteTitle(document.title!), revision: current.page.revision + 1, updatedAt: now() };
    if (page.title !== current.page.title && current.page.parentPageId) state = touchBrowserParents(state, new Set([current.page.parentPageId]));
    state = { ...replacePage(state, page), documents: { ...state.documents, [page.id]: document }, links: semanticLinks(state, document), revisions: pruneBrowserRevisions([...state.revisions, revision]) };
    return synchronizeChildPages(state);
  }

const browserNotesAdapter: NotesGateway = {
  pageCatalog: browserNotePageCatalogGateway,
  loadIndexDocuments: readIndexDocuments,
  applyMetadataBatch: browserNoteMetadataBatch(synchronizeChildPages,saveBrowserDocument),
  replayMetadataBatch: request => browserNoteMetadataBatch(synchronizeChildPages,saveBrowserDocument)({operationId:request.operationId,action:request.direction==="undo"?"restore":"trash",items:request.items},request),
  async listPageMetadata(ids) { const state = await readNotePageState(ids); return state.pages.map(page => presentPage(state, page)); },
  ...browserNoteNavigation({ saveBrowserDocument, snapshot }),
  async listPages(query: NoteListQuery = {}) {
    const needle = query.search?.trim().toLocaleLowerCase() ?? "";
    const state = await readState({metadataOnly: true}), readingPages = new Set(state.readingSources.map(source => source.pageId));
    return state.pages.filter((page) =>
      page.status === (query.status ?? "active")
      && (query.kind === "all" || !readingPages.has(page.id))
      && (query.parentPageId === undefined || page.parentPageId === query.parentPageId)
      && (query.projectId === undefined || page.projectIds.includes(query.projectId))
      && (query.tagId === undefined || page.tagIds.includes(query.tagId))
      && (query.cursor === undefined || page.id > query.cursor)
      && (needle === "" || page.title.toLocaleLowerCase().includes(needle)),
    ).sort((a, b) => query.kind === "all" ? (a.id < b.id ? -1 : a.id > b.id ? 1 : 0) : a.position - b.position).slice(0, query.limit ?? 500).map(page => presentPage(state, page));
  },
  async createPage(draft: NotePageDraft) { return createBrowserPage(draft); },
  async createLiteratureNote(paperId: string) { return createBrowserPage({ title: "文献笔记" }, paperId); },
  ...browserNoteReads(),
  async updatePage(request: NotePageMetadataUpdate) {
    let state = await readState({hbsPageId: request.pageId});
    const current = snapshot(state, request.pageId).page;
    assertRevision(current.revision, request.expectedRevision);
    const reading = readingNotesFromState(state).find(note => note.pageId === current.id);
    if (request.isFavorite === true && reading && isDependentReadingNote(reading)) throw new NotesGatewayError("invalid_input", "附属笔记不支持收藏");
    const page: NotePage = {
      ...current,
      ...(request.title === undefined ? {} : { title: normalizeNoteTitle(request.title) }),
      ...(request.isFavorite === undefined ? {} : { isFavorite: request.isFavorite }),
      ...(request.projectIds === undefined ? {} : { projectIds: [...request.projectIds] }),
      ...(request.tagIds === undefined ? {} : { tagIds: [...request.tagIds] }),
      revision: current.revision + 1, updatedAt: request.title !== undefined && normalizeNoteTitle(request.title) !== current.title ? now() : current.updatedAt,
    };
    state = replacePage(state, page);
    if (request.title !== undefined) state = {...state, documents: {...state.documents, [page.id]: {...state.documents[page.id]!, title: page.title}}};
    const hbs = state.hbsRecords?.[page.id];
    if (request.title !== undefined && hbs?.baseline) state = {...state, hbsRecords: {...state.hbsRecords,
      [page.id]: {...hbs, baseline: {...hbs.baseline, title: page.title}}}};
    if (request.title !== undefined && current.parentPageId !== undefined) state = touchBrowserParents(state, new Set([current.parentPageId]));
    state = synchronizeChildPages(state); await writeState(state); return page;
  },
  async movePage(request: NotePageMoveRequest) {
    let state = await readState();
    const current = snapshot(state, request.pageId).page;
    assertRevision(current.revision, request.expectedRevision);
    if (!Number.isSafeInteger(request.position) || request.position < 0) throw new NotesGatewayError("invalid_input", "页面位置无效");
    if (request.parentPageId !== undefined && !state.pages.some((page) => page.id === request.parentPageId && page.status !== "trashed")) {
      throw new NotesGatewayError("invalid_input", "父页面不存在");
    }
    let cursor = request.parentPageId;
    while (cursor !== undefined) {
      if (cursor === request.pageId) throw new NotesGatewayError("invalid_input", "页面树不能形成环路");
      cursor = state.pages.find((page) => page.id === cursor)?.parentPageId;
    }
    const pages = reparentAndOrder(state.pages, current, request.parentPageId, request.position);
    const page = pages.find((candidate) => candidate.id === current.id);
    if (page === undefined) throw new NotesGatewayError("not_found", "找不到移动后的页面");
    const affectedParents = new Set<NotePageId>();
    if (current.parentPageId !== undefined) affectedParents.add(current.parentPageId);
    if (request.parentPageId !== undefined) affectedParents.add(request.parentPageId);
    state = synchronizeChildPages(touchBrowserParents({ ...state, pages }, affectedParents)); await writeState(state); return page;
  },
  async duplicatePage(pageId) {
    const original = await this.loadDocument(pageId);
    const created = await this.createPage({
      title: `${original.page.title} 副本`,
      ...(original.page.parentPageId === undefined ? {} : { parentPageId: original.page.parentPageId }),
    });
    return this.saveDocument({ document: {...remapDocument(original.document, created.page.id), title: created.page.title}, expectedRevision: 0, saveReason: "structure" });
  },
  async setPageStatus(request) {
    if (!["active", "archived", "trashed"].includes(request.status)) throw new NotesGatewayError("invalid_input", "笔记状态无效");
    const receipt = await this.applyMetadataBatch({operationId: crypto.randomUUID(), action: request.status === 'trashed' ? 'trash' : request.status === 'archived' ? 'archive' : 'restore', items: [{id: request.id, expectedRevision: request.expectedRevision}]});
    const failure = receipt.failures[0];
    if (failure) throw new NotesGatewayError(failure.code as import('@research-assistant/data/notes').NotesErrorCode, failure.message, failure.currentRevision);
    const page = receipt.pages.find(page => page.id === request.id) ?? (await this.listPageMetadata([request.id]))[0];
    if (!page) throw new NotesGatewayError('not_found', '找不到笔记页面');
    return page;
  },
  async permanentlyDeletePage(request) {
    const state = await readState(); const current = snapshot(state, request.id);
    assertRevision(current.page.revision, request.expectedRevision);
    if (current.page.status !== "trashed") throw new NotesGatewayError("invalid_input", "只有回收站中的笔记才能永久删除");
    const documents = { ...state.documents }; delete documents[request.id];
    const timestamp = now();
    const retainedDescendants = new Set(descendantIds(state.pages, request.id));
    retainedDescendants.delete(request.id);
    const pages = state.pages.filter((page) => page.id !== request.id).map((page): NotePage => {
      if (!retainedDescendants.has(page.id)) return page;
      const { parentPageId, trashedAt: _trashedAt, ...withoutLifecycle } = page;
      return {
        ...withoutLifecycle,
        ...(parentPageId === request.id ? {} : { parentPageId }),
        status: "active",
        revision: page.revision + 1,
        updatedAt: timestamp,
      };
    });
    const orderedRoots = pages.filter((page) => page.parentPageId === undefined).sort((a, b) => a.position - b.position || a.id.localeCompare(b.id));
    const rootPositions = new Map(orderedRoots.map((page, index) => [page.id, index]));
    const normalizedPages = pages.map((page): NotePage => {
      const position = rootPositions.get(page.id);
      if (position === undefined || position === page.position) return page;
      return { ...page, position, revision: page.revision + 1, updatedAt: timestamp };
    });
    const next: BrowserNotesState = {
      ...state,
      pages: normalizedPages,
      readingSources: state.readingSources.filter(source => source.pageId !== request.id),
      documents,
      links: state.links.filter((link) => link.sourcePageId !== request.id),
      revisions: state.revisions.filter((revision) => revision.pageId !== request.id),
    };
    await writeState(synchronizeChildPages(next));
    return { scheduledAssetCount: 0, estimatedReclaimableBytes: 0, cleanupWarnings: [] };
  },
  async loadDocument(pageId) { return snapshot(await readNotePageState([pageId],true), pageId); },
  async saveDocument(request: NoteDocumentSaveRequest) {
    const state = saveBrowserDocument(await readState(), request);
    await writeState(state);
    return snapshot(state, request.document.pageId);
  },
  async loadRevision(pageId, revisionId) {
    const revision = (await readState()).revisions.find(value => value.pageId === pageId && value.id === revisionId);
    if (!revision) throw new NotesGatewayError("not_found", "历史版本不存在");
    return revision.document;
  },
  resolveConflict: request => resolveBrowserConflict(request, saveBrowserDocument, snapshot),
  async listRevisions(pageId) { return (await readState()).revisions.filter((revision) => revision.pageId === pageId).map(({ document: _, ...summary }) => summary); },
  async setRevisionPinned(pageId, revisionId, isPinned) {
    const state = await readState();
    const revision = state.revisions.find((candidate) => candidate.id === revisionId && candidate.pageId === pageId);
    if (revision === undefined) throw new NotesGatewayError("not_found", "找不到历史版本");
    const updated = { ...revision, isPinned };
    await writeState({ ...state, revisions: state.revisions.map((candidate) => candidate.id === revisionId ? updated : candidate) });
    const { document: _, ...summary } = updated;
    return summary;
  },
  async restoreRevision(pageId, revisionId, expectedRevision) {
    const revision = (await readState()).revisions.find((candidate) => candidate.id === revisionId && candidate.pageId === pageId);
    if (revision === undefined) throw new NotesGatewayError("not_found", "找不到笔记历史版本");
    return this.saveDocument({ document: revision.document, expectedRevision, saveReason: "restore" });
  },
  async createReadingNote(draft: ReadingNoteDraft) {
    const state = await readState(); const timestamp = now(); const literature = readJsonArray<LiteratureItem>(LITERATURE_KEY);
    const paper = literature.find((item) => item.id === draft.itemId); const attachment = paper?.attachments.find((item) => item.id === draft.attachmentId);
    const anchorId = draft.sourceAnchorId ?? crypto.randomUUID();
    const note: ReadingNoteBlock = {
      id: draft.id ?? noteBlockId(crypto.randomUUID()), noteType: draft.noteType, body: draft.body,
      color: draft.color ?? "#f4cf65", status: "active", isFavorite: false, revision: 0,
      source: { anchorId, paperId: draft.itemId, paperTitle: paper?.title ?? "来源文献", attachmentId: draft.attachmentId,
        contentSha256: attachment?.contentSha256 ?? "", pageIndex: draft.pageIndex, ...(draft.pageLabel === undefined ? {} : { pageLabel: draft.pageLabel }),
        rects: draft.rects, quoteExact: draft.quoteExact, quotePrefix: draft.quotePrefix ?? "", quoteSuffix: draft.quoteSuffix ?? "",
        state: draft.noteType === "page_note" ? "page_only" : "exact",
        ...(draft.selector === undefined ? {} : { selector: { ...draft.selector, anchorId } }) },
      projectIds: [], tagIds: [], createdAt: timestamp, updatedAt: timestamp,
    };
    if (state.readingSources.some(source => source.id === note.id)) throw new NotesGatewayError("invalid_input", "阅读笔记 ID 已存在");
    const pageId = notePageId(crypto.randomUUID());
    const { id, body, source, noteType, color, ...properties } = note;
    const page: NotePage = { ...properties, id: pageId, title: source.paperTitle || "阅读笔记", schemaVersion: 1, position: state.pages.length };
    const document = createNoteDocumentFromText(pageId, body, () => noteBlockId(crypto.randomUUID()));
    await writeState({ ...state, pages: [...state.pages, page], documents: { ...state.documents, [pageId]: document },
      readingSources: [...state.readingSources, { id, pageId, source, noteType, color }] });
    return this.getReadingNote(id);
  },
  async updateReadingNote(request: ReadingNoteUpdate) {
    if ("body" in request) throw new NotesGatewayError("invalid_input", "阅读笔记正文请使用文档保存接口");
    const state = await readState(), current = await this.getReadingNote(request.id);
    assertRevision(current.revision, request.expectedRevision);
    if (request.isFavorite === true && isDependentReadingNote(current)) throw new NotesGatewayError("invalid_input", "附属笔记不支持收藏");
    if (request.position !== undefined && (!isPositionReadingNote(current) || current.source.state !== "exact"
      || !Object.values(request.position).every(Number.isFinite) || request.position.x < 0 || request.position.y < 0
      || request.position.width <= 0 || request.position.height <= 0 || request.position.x + request.position.width > 1 || request.position.y + request.position.height > 1)) {
      throw new NotesGatewayError("invalid_input", "只能移动当前附件中有效的位置笔记");
    }
    const pageId = current.pageId!;
    const page = { ...snapshot(state, pageId).page,
      ...(request.isFavorite === undefined ? {} : { isFavorite: request.isFavorite }),
      ...(request.projectIds === undefined ? {} : { projectIds: [...request.projectIds] }),
      ...(request.tagIds === undefined ? {} : { tagIds: [...request.tagIds] }),
      revision: current.revision + 1, updatedAt: current.updatedAt };
    await writeState({ ...replacePage(state, page), readingSources: state.readingSources.map(record => record.id === request.id ? { ...record,
      ...(request.color === undefined ? {} : { color: request.color }),
      ...(request.position === undefined ? {} : { source: readingNoteSourceAt(record.source, request.position) }) } : record) });
    return this.getReadingNote(request.id);
  },
  async setReadingNoteStatus(request) {
    const note = await this.getReadingNote(request.id);
    await this.setPageStatus({ ...request, id: note.pageId! });
    return this.getReadingNote(request.id);
  },
  async permanentlyDeleteReadingNote(request) {
    const note = await this.getReadingNote(request.id);
    return this.permanentlyDeletePage({ ...request, id: note.pageId! });
  },
  async upsertLink(draft: NoteLinkDraft) {
    const state = await readState(); const existing = state.links.find((link) => link.sourcePageId === draft.sourcePageId && link.sourceBlockId === draft.sourceBlockId && link.target.kind === draft.target.kind && link.target.id === draft.target.id && link.relation === draft.relation);
    const link: NoteLink = existing ?? { id: noteLinkId(crypto.randomUUID()), ...draft, isBroken: false, createdAt: now() };
    await writeState({ ...state, links: [...state.links.filter((candidate) => candidate.id !== link.id), link] }); return link;
  },
  async removeLink(id) { const state = await readState(); await writeState({ ...state, links: state.links.filter((link) => link.id !== id) }); },
  listSourceLabels: browserNoteSourceLabels,
  async listOutgoing(pageId) { const state = await readNoteRelations({pageId}); return withBrokenState(state, state.links); },
  async listBacklinks(targetKind, targetId) {
    return withBrowserNotesWrite(async () => {
      const state = await readNoteRelations({targetKind,targetId});
      const links = state.links.filter(link => link.target.kind === targetKind && link.target.id === targetId);
      const pages = new Set(state.pages.map(page => page.id));
      const missing = links.filter(link => !pages.has(link.sourcePageId));
      await removeMissingSourceLinks(state, missing.map(link => link.id));
      return withBrokenState(state, links.filter(link => pages.has(link.sourcePageId)));
    });
  },
  searchNotes: searchBrowserNotes,
  async rebuildNoteIndex() {},
  async cancelNoteIndexRebuild() { return false; },
  async getNoteIndexState() { return "ready"; },
  async importNoteAsset(request) {
    if (request.bytes.length > 16 * 1024 * 1024) throw new NotesGatewayError("invalid_input", "单个笔记附件不能超过 16 MB");
    const storage = getAppStorage();
    const digest = await crypto.subtle.digest("SHA-256", Uint8Array.from(request.bytes));
    const id = [...new Uint8Array(digest)].map((value) => value.toString(16).padStart(2, "0")).join("");
    const assertOwner = () => {if (getAppStorage() !== storage) throw new NotesGatewayError("operation_cancelled", "笔记存储拥有者已切换，附件未插入");};
    assertOwner();
    const displayName = noteImageDisplayName(id, request.mimeType) ?? request.displayName;
    storage.setItem(`${NOTE_ASSET_PREFIX}${id}`, JSON.stringify({ ...request, id, displayName }));
    await flushStorage(storage);
    assertOwner();
    return { id, displayName, mimeType: request.mimeType, byteSize: request.bytes.length };
  },
  async cancelNoteAssetImport() { return false; },
  async readNoteAsset(id) {
    const raw = getAppStorage().getItem(`${NOTE_ASSET_PREFIX}${id}`);
    if (raw === null) throw new NotesGatewayError("not_found", "找不到笔记附件");
    const value = JSON.parse(raw) as { readonly bytes: readonly number[] };
    return Uint8Array.from(value.bytes);
  },
  async exportNotes() {
    throw new NotesGatewayError("transfer_failed", "页面树与附件导出请在桌面应用中使用");
  },
  async cancelNoteExport() { return false; },
  async listNoteTags() {
    return readJsonArray<{ readonly id: string; readonly name: string }>(NOTE_TAG_KEY);
  },
  async ensureNoteTag(name) {
    const normalized = name.trim();
    if (normalized === "" || normalized.length > 80) throw new NotesGatewayError("invalid_input", "标签名称无效");
    const tags = await this.listNoteTags();
    const existing = tags.find((tag) => tag.name.toLocaleLowerCase() === normalized.toLocaleLowerCase());
    if (existing !== undefined) return existing;
    const tag = { id: crypto.randomUUID(), name: normalized };
    getAppStorage().setItem(NOTE_TAG_KEY, JSON.stringify([...tags, tag]));
    return tag;
  },
};

export const browserNotesGateway = serializeBrowserNotesWrites(browserNotesAdapter);
