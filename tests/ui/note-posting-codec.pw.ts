import {projectPath} from "./projectPaths";
import {test,expect} from "@playwright/test";
import {resolve} from "node:path";

test("v4 sparse indexes upgrade without bodies and mix dense saves with rollback and corrupt-data diagnostics",async ({page}) => {
  await page.goto("/@vite/client");
  const result = await page.evaluate(async file => {const {verifyMixedPostingCodec} = await import("/@fs/" + file);return verifyMixedPostingCodec();},projectPath("tests/ui/note-posting-codec.browser.ts"));
  expect(result.oldClosed).toBe(true);expect(result.oldVersionRejected).toBe(true);expect(result.oldSparse).toBe(true);
  expect(result.canonicalReads).toBe(0);expect(result.diagnosed).toBe(true);
  console.log("mixed-posting-codec",JSON.stringify(result));
});

test("blocked note upgrades close late connections instead of leaking an obsolete writer",async ({page}) => {
  await page.goto("/@vite/client");
  const result = await page.evaluate(async () => {
    const path = "/src/core/features/notes/gateways/browserNotesDatabase.ts", {openIndexedDatabase} = await import(path), name = "note-blocked-codec-" + crypto.randomUUID();
    const {noteStores} = await import("/src/core/features/notes/gateways/browserNotesDatabase.ts");
    const {createCatalogStores} = await import("/src/core/features/notes/gateways/browserNoteCatalogStore.ts");
    const old = await new Promise<IDBDatabase>((resolve,reject) => {const request = indexedDB.open(name,4);request.onupgradeneeded = () => {
      for (const store of [...noteStores,"version"]) request.result.createObjectStore(store);
      createCatalogStores(request.result);
      request.transaction!.objectStore("sources").createIndex("pageId","pageId");
      request.transaction!.objectStore("links").createIndex("sourcePageId","sourcePageId");
      request.transaction!.objectStore("catalogHeaders").createIndex("reading",["item.target.kind","item.id"]);
    };request.onsuccess = () => resolve(request.result);request.onerror = () => reject(request.error);});
    let diagnosed = false, future: IDBDatabase | undefined;
    try {
      try {await openIndexedDatabase(name);} catch (error) {diagnosed = Boolean(error && typeof error === "object" && "code" in error && error.code === "database_busy");}
      old.close();
      future = await new Promise<IDBDatabase>((resolve,reject) => {const request = indexedDB.open(name,9);request.onsuccess = () => resolve(request.result);request.onerror = () => reject(request.error);request.onblocked = () => reject(Error("late abandoned owner leaked"));});
      return {diagnosed,version: future.version};
    } finally {old.close();future?.close();await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);});}
  });
  expect(result).toEqual({diagnosed: true,version: 9});
});
