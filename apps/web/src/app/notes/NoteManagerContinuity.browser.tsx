import {pluginDeclarationFixtures as defaultWorkbench} from "../pluginDeclarations.testSupport";
import {createRoot} from "react-dom/client";
import {createNoteDocumentAdapter} from "@research-assistant/core/hbs";
import {createNoteDocumentFromText, noteBlockId, noteDocumentPlainText, type NoteBlockV1, type NoteDocumentV1} from "@research-assistant/core/notes";
import {NotesClient, NotesProvider} from "../../core/features/notes/NotesClient";
import {getNotesGateway} from "../../core/features/notes/gateways";
import {browserNotesGateway} from "../../core/features/notes/gateways/browserNotesGateway";
import {configureBrowserNotesDatabase, openIndexedDatabase, type BrowserNotesDatabase} from "../../core/features/notes/gateways/browserNotesDatabase";
import {createTestNamespacedStorage} from "../../core/runtime/testing/createTestNamespacedStorage";
import {getAppRuntime, selectAppRuntime} from "../../core/runtime/appRuntime";
import {getHbsDocumentGateway} from "../../core/services/hbsDocumentGateway";
import {CoreNoteWorkspaceHost, RetainedNoteEditors, useNoteDocumentSession} from "../../core/public/noteDocuments";
import {useNoteResources, type NoteResourceAccess} from "../../core/public/noteResources";
import {writeWorkspaceState} from "../../core/public/workspaceState";
import {NotesWorkspace} from "@research-assistant/note-manager-plugin/workspace";
import {BuiltinPluginConfiguration, BuiltinPluginConfigurations} from "../pluginProviders.testSupport";
import {BuiltinAgentExtensions, BuiltinAgentExtensionScope} from "../pluginProviders.testSupport";
import {directTextDiff} from "../../core/components/hbs/codeMirrorHbsPolicy";
import type {NoteEditorInstance} from "../../core/features/notes/editor/noteEditorSchema";

type Lifecycle = "hidden" | "unmounted";
const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
function check(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
async function waitFor(condition: () => boolean, message: () => string) {
  const until = performance.now() + 10_000;
  while (!condition() && performance.now() < until) await frame();
  check(condition(), message());
}

/** Read the existing full App's canonical resource through its selected owner.
 * This adds no mounted tree, route, storage, save path or replacement provider. */
export async function inspectReadingNoteResource(quoteExact: string) {
  const gateway = getNotesGateway();
  const matching = (await gateway.listReadingNotes()).filter(note => note.source.quoteExact === quoteExact);
  check(matching.length === 1 && matching[0]!.pageId, "Expected one persisted reading-note resource");
  const note = matching[0]!, pageId = note.pageId!;
  const snapshot = await gateway.loadDocument(pageId), revisions = await gateway.listRevisions(pageId);
  const catalog = await gateway.pageCatalog!.queryNotePageCatalog({category: "all", view: "recent", search: "", limit: 20});
  const entry = catalog.items.find(item => item.target.kind !== "writing" && item.target.pageId === pageId);
  check(entry, "Reading note is absent from the same Core catalog");
  const assetIds = new Set<string>();
  function collect(blocks: readonly NoteBlockV1[]) {
    for (const block of blocks) {
      if (block.content.kind === "asset") assetIds.add(block.content.assetId);
      collect(block.children);
    }
  }
  collect(snapshot.document.blocks);
  return {noteId: note.id, pageId, catalogTitle: entry.title, revision: snapshot.page.revision, document: snapshot.document,
    history: await Promise.all(revisions.map(async revision => ({id: revision.id, pageRevision: revision.pageRevision,
      document: await gateway.loadRevision(pageId, revision.id)}))),
    assets: await Promise.all([...assetIds].sort().map(async id => ({id, bytes: [...await gateway.readNoteAsset(id)]}))),
  };
}

/** Test-only application assembly. Real manager, editor, note repository, HBS
 * authority, catalog and IndexedDB are used; only native Agent IO is absent. */
export async function verifyNoteManagerContinuity(lifecycle: Lifecycle) {
  const previous = {...getAppRuntime(), storage: getAppRuntime().storage};
  const storage = createTestNamespacedStorage(localStorage, `note-manager-continuity:${crypto.randomUUID()}:`);
  const databases: BrowserNotesDatabase[] = [], names = new Set<string>(), errors: string[] = [];
  const pendingDatabaseWork = new Set<Promise<unknown>>();
  function trackDatabaseWork<T>(operation: Promise<T>): Promise<T> {
    const settled = operation.finally(() => pendingDatabaseWork.delete(settled));
    pendingDatabaseWork.add(settled); return settled;
  }
  configureBrowserNotesDatabase(name => trackDatabaseWork((async () => {
    const database = await openIndexedDatabase(name);
    databases.push(database); names.add(name); return database;
  })().then(database => ({...database,
    read: (...args: Parameters<BrowserNotesDatabase["read"]>) => trackDatabaseWork(database.read(...args)),
    commit: (...args: Parameters<BrowserNotesDatabase["commit"]>) => trackDatabaseWork(database.commit(...args)),
    ...(database.queryCatalog ? {queryCatalog: (...args: Parameters<NonNullable<BrowserNotesDatabase["queryCatalog"]>>) => trackDatabaseWork(database.queryCatalog!(...args))} : {}),
    ...(database.searchNotes ? {searchNotes: (...args: Parameters<NonNullable<BrowserNotesDatabase["searchNotes"]>>) => trackDatabaseWork(database.searchNotes!(...args))} : {}),
  }))));
  const unsupported = async (): Promise<never> => {throw new Error("Unexpected Agent write in note continuity test");};
  selectAppRuntime({mode: "browser", storage, gateways: {textPatchGateway: {
    forPage: async () => [], review: async () => null, decide: unsupported, resolve: unsupported,
  }}, dispose() {}});
  writeWorkspaceState("notes-mode", "independent");
  writeWorkspaceState("notes-scope", "recent");
  writeWorkspaceState("notes-explorer-open", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host), adapter = createNoteDocumentAdapter(directTextDiff);
  let current!: ReturnType<typeof useNoteDocumentSession>, resources!: NoteResourceAccess, editor!: NoteEditorInstance;
  const ready = (value: NoteEditorInstance) => {editor = value;};
  const onError = (message: string) => errors.push(message);
  function CoreDocument() {
    resources = useNoteResources();
    current = useNoteDocumentSession({onError, onNotice() {}, onLoaded() {}});
    return <RetainedNoteEditors sessions={current.retainedSessions} activeId={current.snapshot?.page.id}
      conflictLocked={false} onChange={current.onRetainedDocumentChange} onEditorReady={ready}/>;
  }
  async function showManager(visible: boolean) {
    // hidden+active is the production App host convention. The second case
    // additionally destroys this consumer, while the Core document host stays alive.
    // Mount the observed Core projection last so its real editor-ready callback
    // observes the single retained editor shared with the management view.
    root.render(<BuiltinAgentExtensions selected={[{id: "note_manager"}]} bindings={defaultWorkbench.bindings}><BuiltinPluginConfigurations selected={[{id: "note_manager"}]} registrations={defaultWorkbench.configurations}><NotesProvider gateway={browserNotesGateway}>
      <CoreNoteWorkspaceHost embedded={false} active>
        <div className="notes-workspace-host" hidden={!visible}>
          {(visible || lifecycle === "hidden") && <BuiltinAgentExtensionScope owner="note_manager"><BuiltinPluginConfiguration owner="note_manager"><NotesWorkspace active={visible}/></BuiltinPluginConfiguration></BuiltinAgentExtensionScope>}
        </div>
        <CoreDocument/>
      </CoreNoteWorkspaceHost>
    </NotesProvider></BuiltinPluginConfigurations></BuiltinAgentExtensions>);
    await frame(); await frame();
  }
  let workflowFailure: unknown;
  try {
    await showManager(true);
    const originalResources = resources, history = resources.history;
    const created = await resources.mutate(gateway => gateway.createPage({title: "Shared Core note"})), id = created.page.id;
    const png = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jI1sAAAAASUVORK5CYII="), value => value.charCodeAt(0));
    const asset = await browserNotesGateway.importNoteAsset({displayName: "figure.png", mimeType: "image/png", bytes: [...png]});
    const base = createNoteDocumentFromText(id, "Original body", () => noteBlockId("body"));
    const initial: NoteDocumentV1 = {...base, title: created.page.title, blocks: [...base.blocks, {
      id: noteBlockId("figure"), type: "image", props: {}, children: [], content: {
        kind: "asset", assetId: asset.id, displayName: asset.displayName, mimeType: "image/png", caption: "Shared asset",
      },
    }]};
    const hbs = getHbsDocumentGateway();
    await hbs.commit("note", id, {id: `manager-seed-${lifecycle}`, expectedVersion: created.page.revision, saved: initial, baseline: initial});
    await current.loadPage(id);
    await waitFor(() => Boolean(editor?.prosemirrorView.dom.isConnected), () => "Core editor did not mount");
    const manager = host.querySelector(".notes-workspace"), owner = current.saveControllerRef.current;
    const view = editor.prosemirrorView, editorSessionId = current.editorSessionId;
    const listed = () => manager?.querySelector(".notes-simple-list")?.textContent?.includes("Shared Core note") ?? false;
    await waitFor(listed, () => `Manager did not list the Core resource: ${manager?.textContent}`);
    history.closeGroups(); editor.updateBlock("body", {content: "Input before management closes"}); await frame();
    await showManager(false);
    check(host.querySelector<HTMLElement>(".notes-workspace-host")!.hidden, "manager host stayed visible");
    if (lifecycle === "unmounted") check(!host.querySelector(".notes-workspace"), "manager consumer was not destroyed");
    history.closeGroups(); editor.updateBlock("body", {content: "Edited after management closes"}); await frame();
    check(await current.ensureDurable(true), `Core save failed: ${errors.join("; ")}`); await frame();
    check(editor.prosemirrorView === view && current.saveControllerRef.current === owner && current.editorSessionId === editorSessionId,
      "closing management replaced the editor or document session");
    check(resources === originalResources && resources.history === history, "closing management replaced the Core resource/history owner");
    check(await history.run("undo", undefined, view.dom), "shared history lost Undo"); await frame();
    check(noteDocumentPlainText(current.snapshot!.document).includes("Input before management closes"), "Undo lost the pre-close input");
    check(await current.ensureDurable(true), "Undo save failed");
    check(await history.run("redo", undefined, view.dom), "shared history lost Redo"); await frame();
    check(await current.ensureDurable(true), "Redo save failed"); await frame();
    const persisted = await resources.gateway.loadDocument(id), revisions = await resources.gateway.listRevisions(id);
    check(noteDocumentPlainText(persisted.document).includes("Edited after management closes"), "canonical storage lost the edited body");
    const figure = persisted.document.blocks.find(block => block.id === "figure");
    check(figure?.content.kind === "asset" && figure.content.assetId === asset.id && figure.content.caption === "Shared asset", "asset identity changed");
    const assetBytes = await browserNotesGateway.readNoteAsset(asset.id);
    check(assetBytes.length === png.length && [...assetBytes].every((byte, i) => byte === png[i]), "asset bytes changed");
    const preserved = await hbs.readPreservation("note", id) as {content: NoteDocumentV1};
    check(adapter.equal(preserved.content, persisted.document), "preservation no longer matches the Core document");
    await showManager(true);
    const reopened = host.querySelector(".notes-workspace");
    await waitFor(() => reopened?.querySelector(".notes-simple-list")?.textContent?.includes("Shared Core note") ?? false,
      () => `Reopened manager did not list the original resource: ${reopened?.textContent}`);
    check((reopened === manager) === (lifecycle === "hidden"), "manager lifecycle was not exercised");
    const catalog = await resources.gateway.pageCatalog!.queryNotePageCatalog({category: "independent", view: "recent", search: "", limit: 20});
    check(catalog.items.length === 1 && catalog.items[0]!.target.kind === "page" && catalog.items[0]!.target.pageId === id,
      "manager created or switched to a different note resource");
    check(current.snapshot?.page.id === id && adapter.equal(current.snapshot.document, persisted.document), "reopening management replaced the canonical document");
    check(current.saveControllerRef.current === owner && current.editorSessionId === editorSessionId && editor.prosemirrorView === view,
      "reopening management replaced the editor/session");
    check(resources === originalResources && resources.history === history, "reopening management replaced the Core owner");
    check(JSON.stringify((await resources.gateway.listRevisions(id)).map(revision => revision.id)) === JSON.stringify(revisions.map(revision => revision.id)),
      "reopening management rewrote note history");
    const reopenedAsset = await browserNotesGateway.readNoteAsset(asset.id);
    check(reopenedAsset.length === png.length && [...reopenedAsset].every((byte, i) => byte === png[i]), "reopening management changed the asset");
    check(!errors.length, `Core document errors: ${errors.join("; ")}`);
    return {lifecycle, noteId: id, assetId: asset.id, revision: persisted.page.revision, historyCount: revisions.length,
      managerRetained: reopened === manager, editorRetained: editor.prosemirrorView === view, ownerRetained: resources === originalResources};
  } catch (reason) {
    workflowFailure = reason; throw reason;
  } finally {
    try {
      root.unmount(); await frame(); await frame(); host.remove();
      await waitFor(() => pendingDatabaseWork.size === 0,
        () => `synthetic note database cleanup has ${pendingDatabaseWork.size} accepted operations pending`);
      for (const database of databases) database.close?.();
      configureBrowserNotesDatabase(openIndexedDatabase);
      selectAppRuntime(previous); storage.clear();
      for (const name of names) await new Promise<void>((resolve, reject) => {
        const request = indexedDB.deleteDatabase(name);
        request.onsuccess = () => resolve(); request.onerror = () => reject(request.error);
        request.onblocked = () => reject(new Error("synthetic note database cleanup blocked"));
      });
    } catch (reason) {
      if (workflowFailure !== undefined) throw new AggregateError([workflowFailure, reason],
        `Note continuity failed: ${String(workflowFailure)}; cleanup failed: ${String(reason)}`);
      throw reason;
    }
  }
}

/** Existing production note storage; no alternate index or management UI. */
export async function verifyReadingNoteSearchWorkflow() {
  const gateway = browserNotesGateway;
  const paperId = crypto.randomUUID(), attachmentId = crypto.randomUUID();
  getAppRuntime().storage.setItem("research-assistant.literature.v1", JSON.stringify([{
    id: paperId, title: "Synthetic source", attachments: [{id: attachmentId, contentSha256: "a".repeat(64)}],
  }]));
  const client = new NotesClient(gateway);
  const created = await client.mutate((port: typeof gateway) => port.createReadingNote({
    noteType: "annotation", body: "comment", itemId: paperId, attachmentId,
    pageIndex: 0, rects: [], quoteExact: "quoted",
  }));
  check(created.pageId, "Reading note is missing its Core page identity");
  const original = await gateway.loadDocument(created.pageId);
  const saved = await client.mutate((port: typeof gateway) => port.saveDocument({
    document: {...original.document, blocks: [{...original.document.blocks[0]!,
      content: {kind: "inline", items: [{type: "text", text: "revised", styles: []}]}}]},
    expectedRevision: original.page.revision, saveReason: "autosave",
  }));
  const edited = await gateway.getReadingNote(created.id);
  const query = {search: "revised", paperId, noteType: "annotation" as const};
  const active = await gateway.searchNotes(query);
  const trashed = await client.mutate((port: typeof gateway) => port.setReadingNoteStatus({
    id: edited.id, status: "trashed", expectedRevision: edited.revision,
  }));
  const whileTrashed = await gateway.searchNotes(query);
  const restored = await client.mutate((port: typeof gateway) => port.setReadingNoteStatus({
    id: trashed.id, status: "active", expectedRevision: trashed.revision,
  }));
  const afterRestore = await gateway.searchNotes(query);
  const reopened = await gateway.loadDocument(created.pageId);
  return {id: created.id, pageId: created.pageId, edited, restored,
    savedDocument: saved.document, reopenedDocument: reopened.document,
    active: active.items.map((item: {objectId: string}) => item.objectId),
    whileTrashed: whileTrashed.items.map((item: {objectId: string}) => item.objectId),
    afterRestore: afterRestore.items.map((item: {objectId: string}) => item.objectId)};
}
