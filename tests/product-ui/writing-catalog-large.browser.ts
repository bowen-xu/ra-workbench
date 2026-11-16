import type { Page } from "@playwright/test";
// Synthetic source-adapter acceptance; this does not assert the main UI cutover.
export async function verifyWritingCatalogLarge(page: Page, paths: {
  writingFile: string;
  dataFile: string;
}): Promise<unknown> {
  await page.goto('/@vite/client');
  const fixture = await page.evaluate(async (paths) => {
    const { createIndexedPreviewStorage } = await import('/ra-workbench/apps/web/src/core/runtime/previewIndexedStorage.ts'), { storagePersistence } = await import('/ra-workbench/apps/web/src/core/runtime/storagePersistence.ts');
    const { createBrowserWritingGateway } = await import('/plugins/ra-writing-projects/src/features/writing/gateway/browserWritingGateway.ts'), { writingFileKey } = await import('/ra-workbench/apps/web/src/core/public/writingStorage.ts');
    const { projectWritingCatalogContent, projectWritingCatalogHeader, writingCatalogId } = await import('/ra-workbench/apps/web/src/core/features/writing-storage/writingCatalogProjection.ts');
    const { emptyWritingCatalogState } = await import('/ra-workbench/apps/web/src/core/features/writing-storage/writingCatalogState.ts');
    const { noteSearchGrams } = await import('/@fs/' + paths.dataFile);
    const { loadBrowserNoteCatalogRuntime } = await import('/ra-workbench/apps/web/src/core/features/notes/gateways/browserNoteCatalogRuntime.ts');
    const { createWritingNote } = await import('/@fs/' + paths.writingFile);
    const prefix = 'writing-catalog-large-' + crypto.randomUUID(), name = 'research-assistant.preview.' + encodeURIComponent(prefix), storage = await createIndexedPreviewStorage(prefix), gateway = createBrowserWritingGateway(storage);
    let db;
    try {
      const a = await gateway.createManuscript({ title: '活动项目' }), created = await gateway.createManuscript({ title: '归档项目' }), b = await gateway.archiveManuscript(created.id), runtime = await loadBrowserNoteCatalogRuntime();
      const entries = await Promise.all(Array.from({ length: 2400 }, async (_, i) => {
        const owner = i < 1200 ? a : b, noteId = i === 0 ? '\ue000' : i === 1 ? '😀' : i === 2 ? '𐀀' : 'note-' + i, path = i < 3 ? 'tie-' + i + '/相同.ranote' : 'notes/研究' + i + '.ranote';
        const json = JSON.stringify(createWritingNote(noteId, 'block-' + i, i === 1737 ? '独有 needle-1737 正文' : '普通正文'));
        const sha256 = [...new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(json)))].map(n => n.toString(16).padStart(2, '0')).join('');
        const file = { manuscriptId: owner.id, relativePath: path, kind: 'other', sizeBytes: new TextEncoder().encode(json).length, sha256, modifiedAt: new Date(Date.UTC(2026, 9, 24) + (i < 3 ? 0 : i) * 1000).toISOString() };
        const id = writingCatalogId(owner.id, noteId), header = projectWritingCatalogHeader(file, owner, id, i + 1, runtime), text = projectWritingCatalogContent(json).text, foldedText = runtime.fold(text), grams = [...noteSearchGrams(foldedText)];
        return { key: writingFileKey(owner.id, path), file, body: { relativePath: path, content: json, sha256 }, header, projection: { text, foldedText, grams }, work: { manuscriptId: owner.id, path, sha256, status: owner.status, state: 'ready', id } };
      }));
      const postings = new Map();
      for (const entry of entries)
        for (const [component, grams] of [[entry.header.ordinal * 2, entry.projection.grams], [entry.header.ordinal * 2 + 1, noteSearchGrams(entry.header.metadataText)]])
          for (const gram of grams) {
            if (!postings.has(gram))
              postings.set(gram, []);
            postings.get(gram).push(component);
          }
      db = await new Promise((resolve, reject) => { const r = indexedDB.open(name); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
      await new Promise((resolve, reject) => {
        const tx = db.transaction(['writingFiles', 'writingBodies', 'writingCatalogFiles', 'catalogHeaders', 'catalogBodies', 'catalogPostings', 'catalogCounts', 'catalogState'], 'readwrite');
        for (const entry of entries) {
          tx.objectStore('writingFiles').put(entry.file, entry.key);
          tx.objectStore('writingBodies').put(entry.body, entry.key);
          tx.objectStore('writingCatalogFiles').put(entry.work, entry.key);
          tx.objectStore('catalogHeaders').put(entry.header);
          tx.objectStore('catalogBodies').put(entry.projection, entry.header.item.id);
        }
        for (const [gram, components] of postings) {
          tx.objectStore('catalogPostings').put(Uint32Array.from(components), gram);
          tx.objectStore('catalogCounts').put(components.length, gram);
        }
        const initial = emptyWritingCatalogState();
        tx.objectStore('catalogState').put({ ...initial, generation: 1, nextOrdinal: 2401, collationVersion: runtime.collationVersion, normalizationVersion: runtime.normalizationVersion, writing: { ...initial.writing, counts: { active: { pending: 0, error: 0, ready: 1200 }, archived: { pending: 0, error: 0, ready: 1200 } } } }, 'current');
        tx.oncomplete = resolve;
        tx.onabort = () => reject(tx.error);
      });
      return { prefix, name, a, b };
    }
    finally {
      db?.close();
      storagePersistence(storage).close();
      await storagePersistence(storage).flush();
    }
  }, paths);
  await page.reload();
  return await page.evaluate(async ({ prefix, name, a, b }) => {
    const { createIndexedPreviewStorage } = await import('/ra-workbench/apps/web/src/core/runtime/previewIndexedStorage.ts'), { storagePersistence } = await import('/ra-workbench/apps/web/src/core/runtime/storagePersistence.ts');
    const { createBrowserWritingGateway } = await import('/plugins/ra-writing-projects/src/features/writing/gateway/browserWritingGateway.ts');
    const metrics = { reads: {}, values: {}, keys: {}, writes: {} };
    const reset = () => { for (const group of Object.values(metrics))
      for (const key of Object.keys(group))
        delete group[key]; }, copy = () => structuredClone(metrics), assert = (ok, message) => { if (!ok)
      throw Error(message); };
    const originals = [];
    for (const [prototype, methods] of [[IDBObjectStore.prototype, ['get', 'openCursor', 'put', 'delete']], [IDBIndex.prototype, ['get', 'getKey', 'openCursor', 'openKeyCursor']]])
      for (const method of methods) {
        const original = prototype[method];
        originals.push([prototype, method, original]);
        prototype[method] = function (...args) { const store = this instanceof IDBIndex ? this.objectStore : this, request = original.apply(this, args); if (store.transaction.db.name !== name)
          return request; const group = method === 'put' || method === 'delete' ? metrics.writes : metrics.reads; group[store.name + '.' + method] = (group[store.name + '.' + method] ?? 0) + 1; if (method !== 'put' && method !== 'delete')
          request.addEventListener('success', () => { if (request.result !== undefined && request.result !== null) {
            const values = method === 'getKey' || method === 'openKeyCursor' ? metrics.keys : metrics.values;
            values[store.name] = (values[store.name] ?? 0) + 1;
          } }); return request; };
      }
    const wasmRequests = () => performance.getEntriesByType('resource').filter(row => row.name.includes('research_note_query_wasm')).length;
    let storage;
    try {
      const before = wasmRequests();
      storage = await createIndexedPreviewStorage(prefix);
      const startup = copy(), catalog = createBrowserWritingGateway(storage).noteCatalog, query = { category: 'all', view: 'recent', limit: 20 };
      reset();
      const first = await catalog.queryWritingNoteCatalog(query), cold = copy();
      assert(first.items[0].title === '研究1199' && cold.values.catalogHeaders === 21 && !cold.values.writingBodies && !cold.values.catalogBodies, 'bounded cold page');
      assert(wasmRequests() === before, 'cold metadata query does not initialize Wasm');
      reset();
      await catalog.queryWritingNoteCatalog(query);
      const hot = copy();
      assert(hot.values.catalogHeaders === 21 && !Object.keys(hot.writes).length, 'hot no writes');
      reset();
      const found = await catalog.queryWritingNoteCatalog({ ...query, search: 'needle-1737' }), search = copy();
      assert(found.items.length === 1 && found.items[0].status === 'archived', 'search includes archived source');
      assert(search.values.catalogBodies === 1 && !search.values.writingBodies && search.values.catalogHeaders === 1, 'rare persisted candidate only');
      const { loadBrowserNoteCatalogRuntime } = await import('/ra-workbench/apps/web/src/core/features/notes/gateways/browserNoteCatalogRuntime.ts'), runtime = await loadBrowserNoteCatalogRuntime();
      const encoder = new TextEncoder(), compareBytes = (a, b) => { for (let i = 0; i < Math.min(a.length, b.length); i++)
        if (a[i] !== b[i])
          return a[i] - b[i]; return a.length - b.length; };
      const all = Array.from({ length: 2400 }, (_, i) => { const owner = i < 1200 ? a : b, noteId = i === 0 ? '\ue000' : i === 1 ? '😀' : i === 2 ? '𐀀' : 'note-' + i; return { id: 'writing:' + JSON.stringify([owner.id, noteId]), title: i < 3 ? '相同' : '研究' + i, source: owner.title, category: '附属笔记', updatedAt: Date.UTC(2026, 9, 24) + (i < 3 ? 0 : i) * 1000, status: owner.status }; });
      let cases = 0, maxHeaders = 0;
      for (const field of ['title', 'category', 'source', 'updatedAt'])
        for (const direction of ['ascending', 'descending'])
          for (const view of ['recent', 'archive'])
            for (const category of ['all', 'dependent']) {
              const expected = all.filter(item => item.status === (view === 'recent' ? 'active' : 'archived')).sort((left, right) => { const delta = field === 'updatedAt' ? left.updatedAt - right.updatedAt : compareBytes(runtime.sortKey(left[field]), runtime.sortKey(right[field])); return (direction === 'ascending' ? delta : -delta) || compareBytes(encoder.encode(left.id), encoder.encode(right.id)); });
              reset();
              const result = await catalog.queryWritingNoteCatalog({ ...query, view, category, sort: { field, direction } });
              const head = copy();
              assert(JSON.stringify(result.items.map(item => item.id)) === JSON.stringify(expected.slice(0, 20).map(item => item.id)), 'first order ' + field + direction + view);
              assert(head.values.catalogHeaders === 21 && !head.values.writingBodies && !head.values.catalogBodies, 'first query budget');
              maxHeaders = Math.max(maxHeaders, head.values.catalogHeaders);
              cases++;
              reset();
              const next = await catalog.queryWritingNoteCatalog({ ...query, view, category, sort: { field, direction }, cursor: result.nextCursor }), deep = copy();
              assert(JSON.stringify(next.items.map(item => item.id)) === JSON.stringify(expected.slice(20, 40).map(item => item.id)), 'next order ' + field + direction + view);
              assert(deep.values.catalogHeaders === 22 && !deep.values.writingBodies && !deep.values.catalogBodies, 'cursor query budget');
              maxHeaders = Math.max(maxHeaders, deep.values.catalogHeaders);
              cases++;
            }
      return { rows: 2400, startup, cold, hot, search, cases, maxHeaders, coldWasmRequests: 0 };
    }
    finally {
      if (storage) {
        storagePersistence(storage).close();
        await storagePersistence(storage).flush();
      }
      for (const [prototype, method, original] of originals)
        prototype[method] = original;
      await new Promise((resolve, reject) => { const r = indexedDB.deleteDatabase(name); r.onsuccess = resolve; r.onerror = () => reject(r.error); r.onblocked = () => reject(Error('cleanup blocked')); });
    }
  }, fixture);
}
