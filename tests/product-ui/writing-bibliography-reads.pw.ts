import {expect,test} from "@playwright/test";

test("bibliography metadata reads only imports once, usage reads sources, and status/reopen load no bodies",async ({page}) => {
  const errors: string[] = [];page.on("pageerror",error => errors.push(error.message));
  await page.route("**/__bibliography_probe.html",route => route.fulfill({contentType:"text/html",body:"<!doctype html><html><body></body></html>"}));
  await page.goto("/__bibliography_probe.html");
  // The data harness has no application entry; use Vite's official HMR setup
  // for transitive React imports without mounting the application itself.
  await page.evaluate(() => import("/@id/__x00__@vitejs/plugin-react/preamble"));
  const result = await page.evaluate(async () => {
    const {createIndexedPreviewStorage} = await import("/ra-workbench/apps/web/src/core/runtime/previewIndexedStorage.ts");
    const {storagePersistence} = await import("/ra-workbench/apps/web/src/core/runtime/storagePersistence.ts");
    const {writingRecords} = await import("/ra-workbench/apps/web/src/core/public/writingStorage.ts");
    const {createRecordWritingGateway} = await import("/plugins/ra-writing-projects/src/features/writing/gateway/recordWritingGateway.ts");
    const {getAppRuntime,selectAppRuntime} = await import("/ra-workbench/apps/web/src/core/runtime/appRuntime.ts");
    const previous = {...getAppRuntime(),storage: getAppRuntime().storage};
    const prefix = "bibliography-read-bounds-" + crypto.randomUUID(), name = "research-assistant.preview." + encodeURIComponent(prefix);
    let storage = await createIndexedPreviewStorage(prefix);
    let item = {id:"library",itemType:"journalArticle",title:"Synthetic",abstractText:"",issuedYear:2026,containerTitle:"Journal",volume:"",issue:"",pages:"",doi:"",url:"",language:"",bibtexKey:"auto2026",
      creators:[{familyName:"Example",givenName:"Alice",literalName:"",role:"author"}],tags:[],readingStatus:"unread",attachments:[],projectIds:[],metadataSources:[],createdAt:"now",updatedAt:"now"};
    const select = () => selectAppRuntime({mode:"browser",storage,gateways:{getLiteratureGateway: {list: async () => [],get: async () => item}},dispose(){}});
    const create = () => createRecordWritingGateway(writingRecords(storage),{},storage,[{path:"local.bib",content:"@book{imported2026,title={Imported},year={2026}}"}]);
    const bodies: string[] = [];let checkpoints = 0, headerPoints = 0;
    const get = IDBObjectStore.prototype.get;
    IDBObjectStore.prototype.get = function(...args) {
      if (this.transaction.db.name === name) {
        if (this.name === "writingBodies") bodies.push(JSON.parse(String(args[0]))[1]);
        if (this.name === "writingCheckpoints") checkpoints++;
        if (this.name === "writingFiles") headerPoints++;
      }
      return get.apply(this,args);
    };
    const check = (value: unknown,message: string) => {if (!value) throw Error(message);};
    try {
      select();let gateway = create();const project = await gateway.createManuscript({title:"Bounded bibliography"});
      for (let i = 0; i < 64; i++) await gateway.createFile(project.id,`chapter-${i}.tex`,i === 31 ? "\\cite{auto2026}" : "Synthetic source ".repeat(100));
      await gateway.createFile(project.id,"data.csv","unrelated body");
      bodies.length = 0;
      check((await gateway.listBibliographyEntries(project.id,true)).length === 1,"initial import");
      check(bodies.length === 1 && bodies[0] === "local.bib","adoption loaded non-bib bodies");
      const initialAdoptBodies = bodies.length;
      check((await gateway.listBibliographyEntries(project.id)).length === 1 && bodies.length === initialAdoptBodies,"no automatic entries still triggered source usage reads");
      await gateway.bindCitation(project.id,{literatureItemId:item.id,requestedKey:"auto2026",bibtexEntry:"@article{auto2026,title={Synthetic}}"});
      bodies.length = 0;const revision = (await writingRecords(storage).read([])).revision;
      for (let i = 0; i < 5; i++) check((await gateway.listBibliographyEntries(project.id,true)).length === 2,"all-entry listing lost unused identity");
      item = {...item,title:"Fresh library metadata"};
      check((await gateway.listBibliographyEntries(project.id,true)).find(row => row.libraryItem)?.libraryItem?.title === item.title,"all-entry query cached stale library metadata");
      const repeatedAllBodies = bodies.length;check(repeatedAllBodies === 0,"all-entry query loaded source usage");
      const expected = (await gateway.listFiles(project.id)).find(file => file.relativePath === "references.bib");
      const beforeStatus = headerPoints;
      check((await gateway.getBibliographyStatus(project.id)).contentHash === expected.sha256 && bodies.length === 0,"status read a body or returned a wrong hash");
      check(headerPoints - beforeStatus === 1,"status did not point-read the selected file header");
      check((await writingRecords(storage).read([])).revision === revision,"read-only listing/status changed file authority");
      const used = await gateway.listBibliographyEntries(project.id), usedBodies = bodies.length;
      check(used.length === 2 && usedBodies === 65 && bodies.every(path => /\.tex$/u.test(path)),"usage query did not read precisely the source files");
      await storagePersistence(storage).flush();storagePersistence(storage).close();await storagePersistence(storage).flush();
      bodies.length = 0;storage = await createIndexedPreviewStorage(prefix);select();gateway = create();
      check((await gateway.listBibliographyEntries(project.id,true)).length === 2 && bodies.length === 0,"reopen repeated adoption or source loads");
      check(checkpoints === 0,"metadata query loaded editor recovery");
      return {sourceFiles:65,initialAdoptBodies,repeatedAllBodies,statusBodies:0,statusHeaderReads:1,usedBodies,afterReopenBodies:bodies.length,checkpoints,revisionUnchanged:true};
    } finally {
      IDBObjectStore.prototype.get = get;selectAppRuntime(previous);
      await storagePersistence(storage).flush();storagePersistence(storage).close();await storagePersistence(storage).flush();
      await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);request.onblocked = () => reject(Error("synthetic bibliography cleanup blocked"));});
    }
  });
  expect(result).toEqual({sourceFiles:65,initialAdoptBodies:1,repeatedAllBodies:0,statusBodies:0,statusHeaderReads:1,usedBodies:65,afterReopenBodies:0,checkpoints:0,revisionUnchanged:true});
  expect(errors).toEqual([]);
  console.log("bibliography-read-bounds",JSON.stringify(result));
});
