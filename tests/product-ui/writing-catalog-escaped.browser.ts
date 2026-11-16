import type { Page } from "@playwright/test";
// Synthetic source-adapter acceptance; this does not assert the main UI cutover.
export async function verifyWritingCatalogEscaped(page: Page, paths: {
  writingFile: string;
  dataFile: string;
}): Promise<unknown> {
  await page.goto('/@vite/client');
  return await page.evaluate(async (paths) => {
    const { createIndexedPreviewStorage } = await import('/ra-workbench/apps/web/src/core/runtime/previewIndexedStorage.ts'), { storagePersistence } = await import('/ra-workbench/apps/web/src/core/runtime/storagePersistence.ts');
    const { createBrowserWritingGateway } = await import('/plugins/ra-writing-projects/src/features/writing/gateway/browserWritingGateway.ts'), { createWritingNote } = await import('/@fs/' + paths.writingFile);
    const prefix = 'writing-catalog-escaped-' + crypto.randomUUID(), name = 'research-assistant.preview.' + encodeURIComponent(prefix), storage = await createIndexedPreviewStorage(prefix), gateway = createBrowserWritingGateway(storage);
    const assert = (ok, label) => { if (!ok)
      throw Error(label); };
    try {
      const owner = await gateway.createManuscript({ title: 'Escaped' }), terms = Array.from({ length: 8 }, (_, i) => '\0'.repeat(500) + 'q' + i), search = terms.join(' ');
      for (let i = 0; i < 25; i++)
        await gateway.createFile(owner.id, 'n-' + i + '.ranote', JSON.stringify(createWritingNote('n-' + i, 'b-' + i, search)));
      const query = { category: 'all', view: 'recent', search, limit: 20 }, first = await gateway.noteCatalog.queryWritingNoteCatalog(query);
      assert(first.items.length === 20 && first.nextCursor, 'first page accepted bounded terms');
      const second = await gateway.noteCatalog.queryWritingNoteCatalog({ ...query, cursor: first.nextCursor });
      assert(second.items.length === 5, 'escaped terms paginate');
      const oldBytes = new TextEncoder().encode(JSON.stringify({ schema: 1, generation: first.revision, bucket: 'search-live/updatedAt/descending', terms, id: first.items.at(-1).id })).length;
      const cursorBytes = new TextEncoder().encode(first.nextCursor).length;
      assert(oldBytes > 8192 && cursorBytes < 512 && !first.nextCursor.includes('\\u0000'), 'bounded fingerprint replaces raw terms');
      let scoped = false;
      try {
        await gateway.noteCatalog.queryWritingNoteCatalog({ ...query, category: 'dependent', cursor: first.nextCursor });
      }
      catch (error) {
        scoped = error.code === 'invalid_input';
      }
      assert(scoped, 'query scope binding');
      return { rawQueryBytes: new TextEncoder().encode(search).length, oldCursorBytes: oldBytes, cursorBytes, pages: [first.items.length, second.items.length], scoped };
    }
    finally {
      storagePersistence(storage).close();
      await storagePersistence(storage).flush();
      await new Promise((resolve, reject) => { const r = indexedDB.deleteDatabase(name); r.onsuccess = resolve; r.onerror = () => reject(r.error); r.onblocked = () => reject(Error('cleanup blocked')); });
    }
  }, paths);
}
