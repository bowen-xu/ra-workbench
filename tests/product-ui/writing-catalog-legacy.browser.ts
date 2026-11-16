import type { Page } from "@playwright/test";
// Synthetic source-adapter acceptance; this does not assert the main UI cutover.
export async function verifyWritingCatalogLegacy(page: Page, paths: {
  writingFile: string;
  dataFile: string;
}): Promise<unknown> {
  await page.goto('/@vite/client');
  return await page.evaluate(async (paths) => {
    const { createIndexedPreviewStorage } = await import('/ra-workbench/apps/web/src/core/runtime/previewIndexedStorage.ts'), { storagePersistence } = await import('/ra-workbench/apps/web/src/core/runtime/storagePersistence.ts');
    const { createBrowserWritingGateway } = await import('/plugins/ra-writing-projects/src/features/writing/gateway/browserWritingGateway.ts'), { createWritingRecordStores } = await import('/ra-workbench/apps/web/src/core/features/writing-storage/indexedWritingRecords.ts');
    const { writingFileKey } = await import('/ra-workbench/apps/web/src/core/public/writingStorage.ts'), { createWritingNote } = await import('/@fs/' + paths.writingFile);
    const assert = (ok, message) => { if (!ok)
      throw Error(message); }, spaces = [], names = [];
    const createLegacy = async (count, { missingBody = false, v1 = false, mixed = false } = {}) => {
      const prefix = 'writing-catalog-legacy-' + crypto.randomUUID(), name = 'research-assistant.preview.' + encodeURIComponent(prefix);
      names.push(name);
      const owner = { id: 'project', title: '旧项目', rootPath: 'main.tex', status: 'active', createdAt: '2026-10-24T00:00:00Z', updatedAt: '2026-10-24T00:00:00Z' };
      const files = await Promise.all(Array.from({ length: count }, async (_, i) => { const manuscriptId = mixed && i%2 ? 'archived-project' : owner.id, path = 'n-' + i + '.ranote', content = JSON.stringify(createWritingNote('legacy-' + i, 'b-' + i, 'oldneedle-' + i)); const sha256 = [...new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(content)))].map(n => n.toString(16).padStart(2, '0')).join(''); return { key: writingFileKey(manuscriptId, path), file: { manuscriptId, relativePath: path, kind: 'other', sizeBytes: new TextEncoder().encode(content).length, sha256, modifiedAt: owner.updatedAt }, body: { relativePath: path, content, sha256 } }; }));
      const db = await new Promise((resolve, reject) => { const r = indexedDB.open(name, v1 ? 1 : 2); r.onupgradeneeded = () => { r.result.createObjectStore('entries'); r.result.createObjectStore('metadata'); if (!v1)
        createWritingRecordStores(r.result); }; r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
      const stores = v1 ? ['entries', 'metadata'] : ['entries', 'metadata', 'writingProjects', 'writingFiles', 'writingBodies'];
      await new Promise((resolve, reject) => {
        const tx = db.transaction(stores, 'readwrite');
        tx.objectStore('metadata').put(0, 'revision');
        tx.objectStore('metadata').put(7, 'writingRevision');
        tx.objectStore('entries').put('keep', 'preference');
        if (v1)
          tx.objectStore('entries').put(JSON.stringify({ manuscripts: [[owner.id, { manuscript: owner, files: files.map(row => [row.file.relativePath, row.body]), directories: [], editedAt: [] }]], sessions: [], drafts: [] }), 'writing-files');
        else {
          tx.objectStore('writingProjects').put({ manuscript: owner, directories: [] }, owner.id);
          if(mixed) tx.objectStore('writingProjects').put({manuscript:{...owner,id:'archived-project',status:'archived'},directories:[]},'archived-project');
          for (const row of files) {
            tx.objectStore('writingFiles').put(row.file, row.key);
            if (!missingBody)
              tx.objectStore('writingBodies').put(row.body, row.key);
          }
        }
        tx.oncomplete = resolve;
        tx.onabort = () => reject(tx.error);
      });
      db.close();
      return { prefix, name, owner, files };
    };
    let currentName = '', bodies = 0, bodyKeys = [];
    const originalGet = IDBObjectStore.prototype.get, originalCursor = IDBObjectStore.prototype.openCursor;
    IDBObjectStore.prototype.get = function (...args) { if (this.transaction.db.name === currentName && this.name === 'writingBodies')
      { bodies++; bodyKeys.push(args[0]); } return originalGet.apply(this, args); };
    IDBObjectStore.prototype.openCursor = function (...args) { if (this.transaction.db.name === currentName && this.name === 'writingBodies')
      throw Error('full canonical body scan'); return originalCursor.apply(this, args); };
    const open = async (fixture) => { currentName = fixture.name; bodies = 0; const storage = await createIndexedPreviewStorage(fixture.prefix); spaces.push(storage); return { storage, gateway: createBrowserWritingGateway(storage) }; };
    try {
      const legacy = await createLegacy(12), { storage, gateway } = await open(legacy), catalog = gateway.noteCatalog, query = { category: 'all', view: 'recent', limit: 20 };
      const before = await catalog.queryWritingNoteCatalog(query);
      assert(before.items.length === 0 && before.pendingCount === 12 && bodies === 0, 'upgrade/list does not read old bodies');
      bodies = 0;
      const first = await catalog.prepareWritingNoteCatalog();
      assert(first.processedCount === 8 && first.pendingCount === 4 && bodies === 8, 'bounded first batch');
      bodies = 0;
      const second = await catalog.prepareWritingNoteCatalog();
      assert(second.processedCount === 4 && second.pendingCount === 0 && bodies === 4, 'resumable second batch');
      bodies = 0;
      const noOp = await catalog.prepareWritingNoteCatalog();
      assert(noOp.processedCount === 0 && bodies === 0, 'ready bodies not reopened');
      storagePersistence(storage).close();
      await storagePersistence(storage).flush();
      const reopened = await open(legacy);
      const list = await reopened.gateway.noteCatalog.queryWritingNoteCatalog(query);
      assert(list.items.length === 12 && bodies === 0, 'ready persisted reopen');
      const mixed=await createLegacy(22), mixedOwner=await open(mixed);
      await mixedOwner.gateway.archiveManuscript('project');
      bodies=0;bodyKeys=[];
      const emptyScope=await mixedOwner.gateway.noteCatalog.prepareWritingNoteCatalog({scope:'active'});
      assert(emptyScope.processedCount===0&&emptyScope.pendingCount===0&&bodies===0,'active preparation does not read archived files');
      const scoped=await mixedOwner.gateway.noteCatalog.prepareWritingNoteCatalog({scope:'archived'});
      assert(scoped.processedCount===8&&scoped.pendingCount===14&&bodies===8,'scoped archived batch');
      const split=await createLegacy(22,{mixed:true,missingBody:true}), splitOwner=await open(split);
      bodies=0;bodyKeys=[];
      const archived=await splitOwner.gateway.noteCatalog.prepareWritingNoteCatalog({scope:'archived'});
      assert(archived.processedCount===8&&archived.pendingCount===3&&archived.errorCount===8&&bodies===8&&bodyKeys.every(key=>JSON.parse(key)[0]==='archived-project'),'archive scope reads only archive bodies and counts');
      bodies=0;bodyKeys=[];
      const active=await splitOwner.gateway.noteCatalog.prepareWritingNoteCatalog({scope:'active',retryErrors:true});
      assert(active.processedCount===8&&active.pendingCount===3&&active.errorCount===8&&bodies===8&&bodyKeys.every(key=>JSON.parse(key)[0]==='project'),'active retry does not reset archived errors');
      const archivePage=await splitOwner.gateway.noteCatalog.queryWritingNoteCatalog({...query,view:'archive'});
      assert(archivePage.pendingCount===3&&archivePage.errorCount===8,'other-scope errors and work remain unchanged');
      const missing = await createLegacy(1, { missingBody: true }), missingOwner = await open(missing);
      await missingOwner.gateway.noteCatalog.prepareWritingNoteCatalog();
      const failed = await missingOwner.gateway.noteCatalog.queryWritingNoteCatalog(query);
      assert(failed.errorCount === 1 && failed.issues[0].cause.includes('document_missing'), 'missing body is explicit failure');
      const row = missing.files[0];
      await missingOwner.gateway.saveFile({ manuscriptId: 'project', relativePath: row.file.relativePath, expectedSha256: row.file.sha256, content: row.body.content });
      assert((await missingOwner.gateway.noteCatalog.queryWritingNoteCatalog(query)).errorCount === 0, 'save repairs missing projection/body');
      const v1 = await createLegacy(1, { v1: true }), v1Owner = await open(v1);
      const v1Result = await v1Owner.gateway.noteCatalog.queryWritingNoteCatalog(query);
      assert(v1Result.pendingCount === 1 && v1Result.items.length === 0 && bodies === 0, 'v1 split precedes v3 registration without body reads');
      await v1Owner.gateway.noteCatalog.prepareWritingNoteCatalog();
      assert((await v1Owner.gateway.noteCatalog.queryWritingNoteCatalog(query)).items.length === 1, 'v1 body available on explicit preparation');
      const race = await createLegacy(1), raceOwner = await open(race), originalDigest = crypto.subtle.digest.bind(crypto.subtle);
      let startedResolve, release, held = false;
      const started = new Promise(resolve => { startedResolve = resolve; }), barrier = new Promise(resolve => { release = resolve; });
      crypto.subtle.digest = function (algorithm, data) { const value = originalDigest(algorithm, data); if (!held && new TextDecoder().decode(data) === race.files[0].body.content) {
        held = true;
        return value.then(async (digest) => { startedResolve(); await barrier; return digest; });
      } return value; };
      let raced;
      try {
        const pending = raceOwner.gateway.noteCatalog.prepareWritingNoteCatalog();
        await Promise.race([started, pending.then(() => { throw Error("preparation finished before the controlled hash barrier"); })]);
        await raceOwner.gateway.saveFile({ manuscriptId: 'project', relativePath: race.files[0].file.relativePath, expectedSha256: race.files[0].file.sha256, content: JSON.stringify(createWritingNote('legacy-0', 'b-0', 'new-race-needle')) });
        release();
        raced = await pending;
      }
      finally {
        release?.();
        crypto.subtle.digest = originalDigest;
      }
      assert(raced.processedCount === 0 && raced.pendingCount === 0, 'concurrent save invalidates prepared old bytes');
      assert((await raceOwner.gateway.noteCatalog.queryWritingNoteCatalog({ ...query, search: 'new-race-needle' })).items.length === 1 && (await raceOwner.gateway.noteCatalog.queryWritingNoteCatalog({ ...query, search: 'oldneedle-0' })).items.length === 0, 'search publishes only new save');
      const future = await new Promise<IDBDatabase>((resolve, reject) => {
        // v6 adds indexed counts; v7 deliberately represents a future owner.
        const request = indexedDB.open(race.name, 7);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
        request.onblocked = () => reject(Error('version change should release the old owner'));
      });
      future.close();
      let closed = false;
      try { await raceOwner.gateway.noteCatalog.queryWritingNoteCatalog(query); }
      catch (error) { closed = error.code === 'writing_storage_closed'; }
      assert(closed, 'old source owner reports a stable closed code');
      return { upgradeBodies: 0, first, second, noOp, persistedRows: 12, scoped: {emptyScope, archived, active}, missingBody: true, v1ToV3: true, concurrentSave: raced, versionChangeClosed: closed };
    }
    finally {
      IDBObjectStore.prototype.get = originalGet;
      IDBObjectStore.prototype.openCursor = originalCursor;
      for (const storage of spaces) {
        storagePersistence(storage).close();
        await storagePersistence(storage).flush();
      }
      for (const name of names)
        await new Promise((resolve, reject) => { const r = indexedDB.deleteDatabase(name); r.onsuccess = resolve; r.onerror = () => reject(r.error); r.onblocked = () => reject(Error('cleanup blocked')); });
    }
  }, paths);
}
