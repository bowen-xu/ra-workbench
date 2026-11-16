import type { Page } from "@playwright/test";
// Synthetic source-adapter acceptance; this does not assert the main UI cutover.
export async function verifyWritingCatalogFlow(page: Page, paths: {
  writingFile: string;
  dataFile: string;
}): Promise<unknown> {
  await page.goto('/@vite/client');
  return await page.evaluate(async (paths) => {
    const { createIndexedPreviewStorage } = await import('/ra-workbench/apps/web/src/core/runtime/previewIndexedStorage.ts');
    const { storagePersistence } = await import('/ra-workbench/apps/web/src/core/runtime/storagePersistence.ts');
    const { writingRecords, writingFileKey } = await import('/ra-workbench/apps/web/src/core/public/writingStorage.ts');
    const { createBrowserWritingGateway } = await import('/plugins/ra-writing-projects/src/features/writing/gateway/browserWritingGateway.ts');
    const { createWritingNote } = await import('/@fs/' + paths.writingFile);
    const prefix = 'writing-catalog-flow-' + crypto.randomUUID(), name = 'research-assistant.preview.' + encodeURIComponent(prefix);
    const metrics = { reads: {}, rows: {}, writes: {} };
    const clear = () => { for (const group of Object.values(metrics))
      for (const key of Object.keys(group))
        delete group[key]; };
    const capture = () => structuredClone(metrics), assert = (ok, message) => { if (!ok)
      throw Error(message); };
    const originals = [];
    for (const [prototype, methods] of [[IDBObjectStore.prototype, ['get', 'getAll', 'getKey', 'getAllKeys', 'openCursor', 'put', 'delete']], [IDBIndex.prototype, ['get', 'getKey', 'getAllKeys', 'openCursor', 'openKeyCursor']]])
      for (const method of methods) {
        const original = prototype[method];
        originals.push([prototype, method, original]);
        prototype[method] = function (...args) { const store = this instanceof IDBIndex ? this.objectStore : this, request = original.apply(this, args); if (store.transaction.db.name !== name)
          return request; const target = method === 'put' || method === 'delete' ? metrics.writes : metrics.reads; target[store.name + '.' + method] = (target[store.name + '.' + method] ?? 0) + 1; if (method !== 'put' && method !== 'delete')
          request.addEventListener('success', () => { if (request.result !== undefined && request.result !== null)
            metrics.rows[store.name] = (metrics.rows[store.name] ?? 0) + (method === 'getAll' || method === 'getAllKeys' ? request.result.length : 1); }); return request; };
      }
    let storage;
    try {
      storage = await createIndexedPreviewStorage(prefix);
      let gateway = createBrowserWritingGateway(storage), catalog = gateway.noteCatalog;
      const project = await gateway.createManuscript({ title: '归属项目' }), inputs = [];
      for (let i = 0; i < 25; i++) {
        const content = JSON.stringify(createWritingNote('note-' + i, 'block-' + i, i === 17 ? 'unique-needle-1737' : '共同正文 ' + i));
        inputs.push(await gateway.createFile(project.id, 'notes/研究' + i + '.ranote', content));
      }
      const query = { category: 'all', view: 'recent', search: '', limit: 20 };
      clear();
      const first = await catalog.queryWritingNoteCatalog(query), firstReads = capture();
      assert(first.items.length === 20 && first.pendingCount === 0 && first.errorCount === 0, 'first page/readiness');
      assert(firstReads.rows.catalogHeaders === 21 && !firstReads.rows.writingBodies && !firstReads.rows.catalogBodies && !Object.keys(firstReads.writes).length, 'bounded metadata only');
      clear();
      const second = await catalog.queryWritingNoteCatalog({ ...query, cursor: first.nextCursor }), secondReads = capture();
      assert(second.items.length === 5 && secondReads.rows.catalogHeaders === 6, 'next cursor page');
      clear();
      const search = await catalog.queryWritingNoteCatalog({ ...query, search: 'unique-needle-1737' }), searched = capture();
      assert(search.items.length === 1 && search.items[0].target.path === 'notes/研究17.ranote', 'literal search');
      assert(!searched.rows.writingBodies && searched.rows.catalogBodies === 1, 'search persisted candidate text only');
      clear();
      await gateway.updateManuscript({ manuscriptId: project.id, title: '修改归属' });
      const ownerUpdate = capture();
      assert(!ownerUpdate.rows.writingBodies && !ownerUpdate.rows.catalogBodies, 'owner metadata edit has no body reads');
      const oldId = search.items[0].id;
      clear();
      await gateway.renameFile(project.id, 'notes/研究17.ranote', 'notes/新标题17.ranote');
      const rename = capture();
      assert(rename.rows.writingBodies === 1 && !rename.rows.catalogBodies, 'file move reads only the moved canonical body');
      const renamed = (await catalog.queryWritingNoteCatalog({ ...query, search: 'unique-needle-1737' })).items[0];
      assert(renamed.id === oldId && renamed.title === '新标题17' && renamed.owner.label === '修改归属', 'rename retains identity/owner');
      let stale = false;
      try {
        await catalog.queryWritingNoteCatalog({ ...query, cursor: first.nextCursor });
      }
      catch (error) {
        stale = error.code === 'revision_conflict';
      }
      assert(stale, 'old cursor invalidated');
      const before = await gateway.readFile(project.id, 'notes/新标题17.ranote');
      clear();
      const saved = await gateway.saveFile({ manuscriptId: project.id, relativePath: before.relativePath, expectedSha256: before.sha256, content: JSON.stringify(createWritingNote('note-17', 'block-17', 'new-unique-token')) });
      const savedReads = capture();
      assert(!savedReads.rows.writingBodies && savedReads.rows.catalogBodies === 1, 'save uses received bytes and one old projection');
      assert((await catalog.queryWritingNoteCatalog({ ...query, search: 'unique-needle-1737' })).items.length === 0, 'old postings removed');
      assert((await catalog.queryWritingNoteCatalog({ ...query, search: 'new-unique-token' })).items.length === 1, 'new postings published');
      await gateway.createFile(project.id, 'duplicate.ranote', saved.content);
      let incomplete = await catalog.queryWritingNoteCatalog(query);
      assert(incomplete.errorCount === 1 && incomplete.issues[0].cause.includes('identity_duplicate'), 'duplicate retained as explicit error');
      await gateway.createFile(project.id, 'invalid.ranote', '{ SYNTHETIC_PRIVATE_PAYLOAD');
      incomplete = await catalog.queryWritingNoteCatalog(query);
      assert(incomplete.errorCount === 2 && !JSON.stringify(incomplete.issues).includes('SYNTHETIC_PRIVATE_PAYLOAD'), 'invalid source preserves safe error');
      assert((await gateway.readFile(project.id, 'invalid.ranote')).content === '{ SYNTHETIC_PRIVATE_PAYLOAD', 'invalid canonical file preserved');
      await gateway.deleteFile(project.id, 'notes/新标题17.ranote');
      clear();
      const retried = await catalog.prepareWritingNoteCatalog({ retryErrors: true }), retryReads = capture();
      assert(retried.processedCount === 2 && retried.errorCount === 1 && retryReads.rows.writingBodies === 2, 'explicit retry only errors');
      assert((await catalog.queryWritingNoteCatalog({ ...query, search: 'new-unique-token' })).items[0].target.path === 'duplicate.ranote', 'duplicate becomes available after original deletion');
      const records = writingRecords(storage), key = writingFileKey(project.id, 'duplicate.ranote'), header = (await gateway.listFiles(project.id)).find(file => file.relativePath === 'duplicate.ranote');
      const snapshot = await records.read([]), beforeRollback = await catalog.queryWritingNoteCatalog(query);
      let rollback = false;
      try {
        await records.commit(snapshot.revision, [{ store: 'writingFiles', key, value: header }, { store: 'writingBodies', key, value: { ...saved, relativePath: 'duplicate.ranote' } }, { store: 'writingSessions', key: 'bad', value: () => { } }]);
      }
      catch (error) {
        rollback = error.message.includes('DataCloneError');
      }
      assert(rollback, 'transaction aborted with original cause');
      assert((await catalog.queryWritingNoteCatalog(query)).revision === beforeRollback.revision, 'catalog rollback generation');
      await gateway.archiveManuscript(project.id);
      assert((await catalog.queryWritingNoteCatalog(query)).items.length === 0, 'active source removed on archive');
      assert((await catalog.queryWritingNoteCatalog({ ...query, view: 'archive' })).items.length === 20, 'archived source queried');
      assert((await catalog.queryWritingNoteCatalog({ ...query, category: 'independent' })).pendingCount === 0, 'unrelated browse scope no readiness errors');
      storagePersistence(storage).close();
      await storagePersistence(storage).flush();
      storage = await createIndexedPreviewStorage(prefix);
      gateway = createBrowserWritingGateway(storage);
      catalog = gateway.noteCatalog;
      clear();
      const reopened = await catalog.queryWritingNoteCatalog({ ...query, view: 'archive' }), reopenReads = capture();
      assert(reopened.items.length === 20 && !reopenReads.rows.writingBodies && reopenReads.rows.catalogHeaders === 21, 'bounded persisted reopen');
      const postingCounts = async () => {
        const connection = await new Promise<IDBDatabase>((resolve,reject) => {const request=indexedDB.open(name);request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);});
        try {
          const names=['catalogPostingRows','catalogPostings','catalogCounts'],tx=connection.transaction(names,'readonly');
          const done=new Promise<void>((resolve,reject)=>{tx.oncomplete=()=>resolve();tx.onerror=tx.onabort=()=>reject(tx.error);});
          const counts=await Promise.all(names.map(store=>new Promise<number>((resolve,reject)=>{const request=tx.objectStore(store).count();request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);})));await done;return counts;
        } finally {connection.close();}
      };
      assert((await postingCounts())[0]>0,'positive indexed posting reset control');
      await storagePersistence(storage).reset();
      storagePersistence(storage).close();
      await storagePersistence(storage).flush();
      storage = await createIndexedPreviewStorage(prefix);
      assert((await createBrowserWritingGateway(storage).noteCatalog.queryWritingNoteCatalog(query)).items.length === 0, 'reset reinitializes empty source state');
      assert((await postingCounts()).every(count=>count===0),'reset leaves no new or legacy posting/count rows');
      return { first: firstReads, second: secondReads, search: searched, ownerUpdate, rename, save: savedReads, retry: retryReads, reopen: reopenReads, stale, rollback };
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
  }, paths);
}
