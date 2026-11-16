import {expect, test} from "@playwright/test";
import type {BrowserNotesDatabase, NoteRead, NoteDocumentSelector} from "../../apps/web/src/core/features/notes/gateways/browserNotesDatabase";

test("search reads one persistent catalog snapshot while an external write waits, then reopens the committed change", async ({page}) => {
  await page.goto("/");
  await expect(page.getByRole("button", {name: "文献", exact: true})).toBeVisible();
  const result = await page.evaluate(async () => {
    const databaseModule = "/src/core/features/notes/gateways/browserNotesDatabase.ts", searchModule = "/src/core/features/notes/gateways/browserNoteSearch.ts";
    const {openIndexedDatabase, configureBrowserNotesDatabase} = await import(/* @vite-ignore */ databaseModule);
    const {searchBrowserNotes} = await import(/* @vite-ignore */ searchModule);
    const token = crypto.randomUUID(); localStorage.setItem("research-assistant.notes.v1.indexeddb", token);
    const name = "research-assistant.notes." + token, db = await openIndexedDatabase(name);
    const document = (text: string) => ({schemaVersion: 1, pageId: "page-a", blocks: [{id: "block", type: "paragraph", props: {}, children: [],
      content: {kind: "inline", items: [{type: "text", text, styles: []}]}}]});
    const page = {id: "page-a", title: "Synthetic A", status: "active", position: 0, schemaVersion: 1, revision: 0,
      isFavorite: false, projectIds: [], tagIds: [], createdAt: "2026-11-13T00:00:00.000Z", updatedAt: "2026-11-13T00:00:00.000Z"};
    const source = {id: "reading-a", pageId: "page-a", noteType: "highlight", color: "green", source: {paperId: "paper-a", paperTitle: "Paper A", attachmentId: "pdf",
      contentSha256: "a".repeat(64), pageIndex: 0, rects: [], quoteExact: "initial quote", quotePrefix: "", quoteSuffix: "", state: "exact"}};
    await db.commit(0, [{store: "pages", key: page.id, value: page}, {store: "documents", key: page.id, value: document("initial visible")},
      {store: "sources", key: source.id, value: source}, {store: "pages", key: "page-b", value: {...page, id: "page-b"}},
      {store: "sources", key: "reading-b", value: {...source, id: "reading-b", pageId: "page-b", source: {...source.source, paperId: "other"}}},
      {store: "documents", key: "page-b", value: {damaged: true}}, {store: "navigation", key: "import", value: {ready: true, fingerprint: null}}]);
    let armed = true, queued: Promise<void> | undefined, documentReads = 0, searchCalls = 0;
    const queries: string[][] = [], get = IDBObjectStore.prototype.get;
    IDBObjectStore.prototype.get = function (...args) {
      const request = get.apply(this, args);
      if (this.transaction.db.name !== name) return request;
      if (this.name === "documents") documentReads++;
      if (this.name === "pages" && args[0] === page.id && this.transaction.mode === "readonly"
        && this.transaction.objectStoreNames.contains("catalogHeaders")) {
        request.addEventListener("success", () => {
          if (!armed) return;
          armed = false;
          // Queue the real writer after the search snapshot is already open.
          queued = db.commit(1, [{store: "pages", key: page.id, value: {...page, status: "archived", revision: 1}},
            {store: "sources", key: source.id, value: {...source, source: {...source.source, quoteExact: "committed quote"}}},
            {store: "documents", key: page.id, value: document("external committed")}]);
        });
      }
      return request;
    };
    try {
      configureBrowserNotesDatabase(async (databaseName: string): Promise<BrowserNotesDatabase> => {
        const actual: BrowserNotesDatabase = await openIndexedDatabase(databaseName);
        const search = actual.searchNotes?.bind(actual);
        if (!search) throw new Error("Persistent search adapter required");
        return {...actual, searchNotes: query => {searchCalls++; return search(query);},
          read: (query: NoteRead, selector?: NoteDocumentSelector) => {queries.push(Object.keys(query)); return actual.read(query, selector);}};
      });
      const query = {objectKinds: ["reading_note"], paperId: "paper-a"};
      const first = await searchBrowserNotes(query);
      if (!queued) throw new Error("Search did not open the expected snapshot");
      await queued;
      const active = await searchBrowserNotes(query), archived = await searchBrowserNotes({...query, status: "archived"});
      return {first, active, archived, queries, documentReads, searchCalls};
    } finally {IDBObjectStore.prototype.get = get;}
  });
  expect(result.first.items.map((item: {snippet: string}) => item.snippet)).toEqual(["initial quote"]);
  expect(result.active.items).toEqual([]);
  expect(result.archived.items.map((item: {snippet: string}) => item.snippet)).toEqual(["external committed"]);
  expect(result.first.items[0].source.quoteExact).toBe("initial quote");
  expect(result.archived.items[0].source.quoteExact).toBe("committed quote");
  expect(result.queries).toEqual([["navigation"]]);
  expect(result.documentReads).toBe(0);
  expect(result.searchCalls).toBe(3);
  await page.reload();
  await expect(page.getByRole("button", {name: "文献", exact: true})).toBeVisible();
  const reopened = await page.evaluate(async () => {
    const module = "/src/core/features/notes/gateways/browserNoteSearch.ts";
    return (await import(/* @vite-ignore */ module)).searchBrowserNotes({objectKinds: ["reading_note"], paperId: "paper-a", status: "archived"});
  });
  expect(reopened.items.map((item: {snippet: string}) => item.snippet)).toEqual(["external committed"]);
  expect(reopened.items[0].source.quoteExact).toBe("committed quote");
});

test("a cold browser search waits for import verification and uses it only for that call", async ({page}) => {
  await page.goto("/");
  await expect(page.getByRole("button", {name: "文献", exact: true})).toBeVisible();
  const result = await page.evaluate(async () => {
    const databaseModule = "/src/core/features/notes/gateways/browserNotesDatabase.ts", searchModule = "/src/core/features/notes/gateways/browserNoteSearch.ts";
    const {openIndexedDatabase, configureBrowserNotesDatabase} = await import(/* @vite-ignore */ databaseModule);
    const {searchBrowserNotes} = await import(/* @vite-ignore */ searchModule);
    const page = {id: "synthetic", title: "Synthetic cold search", status: "active", position: 0, schemaVersion: 1, revision: 0,
      isFavorite: false, projectIds: [], tagIds: [], createdAt: "2026-11-13T00:00:00.000Z", updatedAt: "2026-11-13T00:00:00.000Z"};
    const document = {schemaVersion: 1, pageId: page.id, blocks: [{id: "block", type: "paragraph", props: {}, children: [],
      content: {kind: "inline", items: [{type: "text", text: "cold marker", styles: []}]}}]};
    localStorage.setItem("research-assistant.notes.v1", JSON.stringify({schemaVersion: 2, pages: [page], documents: {[page.id]: document}, readingSources: [], revisions: [], links: []}));
    const queries: string[][] = [];
    let searchCalls = 0;
    configureBrowserNotesDatabase(async (name: string): Promise<BrowserNotesDatabase> => {
      const db: BrowserNotesDatabase = await openIndexedDatabase(name);
      const search = db.searchNotes?.bind(db);
      if (!search) throw new Error("Persistent search adapter required");
      return {...db, searchNotes: query => {searchCalls++; return search(query);},
        read: (query: NoteRead, selector?: NoteDocumentSelector) => {queries.push(Object.keys(query)); return db.read(query, selector);}};
    });
    const first = await searchBrowserNotes({search: "cold marker"}), initialQueries = queries.splice(0);
    const db = await openIndexedDatabase("research-assistant.notes." + localStorage.getItem("research-assistant.notes.v1.indexeddb"));
    const stored = await db.read({navigation: ["import"]});
    const second = await searchBrowserNotes({search: "cold marker"});
    return {first, second, initialQueries, laterQueries: queries, ready: stored.rows.navigation?.import, searchCalls};
  });
  expect(result.first.items.map((item: {objectId: string}) => item.objectId)).toEqual(["synthetic"]);
  expect(result.second).toEqual(result.first);
  expect(result.ready).toMatchObject({ready: true});
  expect(result.initialQueries).toHaveLength(2);
  expect(result.initialQueries[0]).toEqual(["navigation"]);
  expect(result.initialQueries[1]).toContain("documents");
  expect(result.laterQueries).toEqual([]);
  expect(result.searchCalls).toBe(2);
});
