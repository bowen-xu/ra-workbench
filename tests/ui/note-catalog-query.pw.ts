import {projectPath, publicPackagePath} from "./projectPaths";
import {test, expect} from '@playwright/test';
import {resolve} from 'node:path';
import {execFileSync} from 'node:child_process';

test('persistent note catalog bounds reads and atomically maintains search', async ({page}) => {
  test.setTimeout(120_000);
  execFileSync(process.execPath,[projectPath("scripts/prepare-note-query.mjs")],{stdio:'pipe'});
  await page.goto('/@vite/client');
  const result = await page.evaluate(async (coreFile) => {
    const gateway = '/src/core/features/notes/gateways/';
    const {loadBrowserNoteCatalogRuntime} = await import(gateway + 'browserNoteCatalogRuntime.ts');
    const store = await import(gateway + 'browserNoteCatalogStore.ts');
    const {createIndexedPostingStore} = await import(gateway+'browserCatalogPostingRows.ts');
    const {applyBrowserNoteCatalogChanges: apply} = await import(gateway + 'browserNoteCatalogWrites.ts');
    const core = await import('/@fs/' + coreFile);
    const runtime = await loadBrowserNoteCatalogRuntime();
    const name = 'note-catalog-store-probe-' + crypto.randomUUID();
    let db;
    const metrics = {storeReads: {}, rows: {}, keyRows: {}, writes: {}};
    const reset = () => {for (const group of Object.values(metrics)) for (const key of Object.keys(group)) delete group[key];};
    const originals = [];
    for (const [prototype, methods] of [[IDBObjectStore.prototype,['get','getAll','openCursor','put','delete']], [IDBIndex.prototype,['get','getAll','getKey','openCursor','openKeyCursor']]]) for (const method of methods) {
      const original = prototype[method]; originals.push([prototype,method,original]);
      prototype[method] = function(...args) {
        const objectStore = this instanceof IDBIndex ? this.objectStore : this;
        const request = original.apply(this,args);
        if (objectStore.transaction.db.name !== name) return request;
        const group = method === 'put' || method === 'delete' ? metrics.writes : metrics.storeReads;
        const key = objectStore.name + '.' + method;
        group[key] = (group[key] ?? 0) + 1;
        if (method !== 'put' && method !== 'delete') request.addEventListener('success', () => {
          const result = request.result;
          if (result === undefined || result === null) return;
          const values = method === 'openKeyCursor' || method === 'getKey' ? metrics.keyRows : metrics.rows;
          values[objectStore.name] = (values[objectStore.name] ?? 0) + (Array.isArray(result) && method === 'getAll' ? result.length : 1);
        });
        return request;
      };
    }
    const assert = (value,message) => {if (!value) throw Error(message);};
    const open = () => new Promise((resolve,reject) => {
      const request = indexedDB.open(name,1);
      request.onupgradeneeded = () => {
        const database = request.result;
        for (const key of ['pages','documents','sources','links','navigation']) database.createObjectStore(key);
        request.transaction.objectStore('sources').createIndex('pageId','pageId');
        request.transaction.objectStore('links').createIndex('sourcePageId','sourcePageId');
        store.createCatalogStores(database);
        createIndexedPostingStore(database);

      };
      request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
    });
    const commit = async (changes, {rollback=false,ready=false}={}) => {
      const tx = db.transaction([...store.browserCatalogStores,'pages','documents','sources','links','navigation'],'readwrite');
      const done = store.catalogTransactionDone(tx);
      try {
        await apply(tx,changes,runtime);
        for (const change of changes) {const target = tx.objectStore(change.store); if (change.value===undefined) target.delete(change.key); else target.put(change.value,change.key);}
        if (ready) {const state=await store.catalogRequest(tx.objectStore('catalogState').get('current'));tx.objectStore('catalogState').put({...state,searchReady:true},'current');}
        if (rollback) tx.abort();
        await done;
      } catch(error) { try{tx.abort();}catch{} await done.catch(()=>{});if(!rollback)throw error; }
    };
    try {
      db=await open();
      const pages=[],changes=[];
      for(let index=0;index<2400;index++){
        const id=core.notePageId('n-'+String(index).padStart(4,'0'));
        const value={id,title:'研究'+index,position:index,status:'active',isFavorite:false,revision:0,schemaVersion:1,projectIds:[],tagIds:[],createdAt:'2026-10-23T00:00:00Z',updatedAt:new Date(Date.UTC(2026,9,23)+index*1000).toISOString()};
        pages.push(value);
        changes.push({store:'pages',key:id,value},{store:'documents',key:id,value:core.createNoteDocumentFromText(id,index===1737?'独有 needle-1737 正文':'普通正文 '+index,()=>core.noteBlockId(crypto.randomUUID()))});
      }
      const started=performance.now(); await commit(changes,{ready:true}); const seedMs=performance.now()-started;
      const query={category:'all',view:'recent',search:'',limit:20};
      reset();const first=await store.queryBrowserNoteCatalog(db,query);const cold=structuredClone(metrics);
      assert(first.items.length===20 && first.items[0].title==='研究2399','first page/order');
      assert(!cold.rows.documents && cold.rows.catalogHeaders===21 && !cold.rows.catalogBodies,'bounded first metadata page');
      reset();const second=await store.queryBrowserNoteCatalog(db,{...query,cursor:first.nextCursor}); const pagination=structuredClone(metrics);
      assert(second.items[0].title==='研究2379' && pagination.rows.catalogHeaders===22,'cursor/header lookup plus 21');
      reset();const search=await store.queryBrowserNoteCatalog(db,{...query,search:'needle-1737'},runtime);const searched=structuredClone(metrics);
      assert(search.items.length===1 && search.items[0].title==='研究1737','rare literal search');
      assert(!searched.rows.documents && searched.rows.catalogHeaders===1 && searched.rows.catalogBodies===1,'only candidate metadata and derived text');
      reset();const renamed={...pages[1737],title:'新标题1737',revision:1};
      await commit([{store:'pages',key:renamed.id,value:renamed}]);const metadataUpdate=structuredClone(metrics);
      assert(!metadataUpdate.rows.documents && !metadataUpdate.rows.catalogBodies,'metadata edit never reads body projection');
      assert(!metadataUpdate.storeReads['navigation.get'],'metadata edit reuses saved favorite position');
      const beforeRollback=await store.queryBrowserNoteCatalog(db,query);
      await commit([{store:'pages',key:renamed.id,value:{...renamed,title:'不可发布的标题',revision:2}}],{rollback:true});
      const afterRollback=await store.queryBrowserNoteCatalog(db,query);
      assert(beforeRollback.revision===afterRollback.revision,'rollback preserves generation');
      const found=await store.queryBrowserNoteCatalog(db,{...query,search:'不可发布的标题'},runtime);assert(found.items.length===0,'rolled-back postings absent');
      let stale=false;try{await store.queryBrowserNoteCatalog(db,{...query,cursor:first.nextCursor});}catch(error){stale=error.code==='revision_conflict';}assert(stale,'stale cursor rejected');
      db.close();db=await open();reset();const reopened=await store.queryBrowserNoteCatalog(db,query);const reopen=structuredClone(metrics);
      assert(reopened.items[0].title==='研究2399' && !reopen.rows.documents && reopen.rows.catalogHeaders===21,'persisted reopen');
      const byTitle=await store.queryBrowserNoteCatalog(db,{...query,sort:{field:'title',direction:'ascending'}});
      assert(byTitle.items[0].title==='新标题1737' || byTitle.items[0].title==='研究0','Chinese numeric title order');
      return {seedMs,first: cold,pagination,search:searched,metadataUpdate,reopen,rollback:true,staleCursor:true,rows:2400};
    } finally {
      if(db)db.close();
      for(const [prototype,method,original]of originals)prototype[method]=original;
      await new Promise((resolve,reject)=>{const request=indexedDB.deleteDatabase(name);request.onsuccess=resolve;request.onerror=()=>reject(request.error);request.onblocked=()=>reject(Error('probe delete blocked'));});
    }
  },publicPackagePath("@research-assistant/core/notes"));
  expect(result.rows).toBe(2400);
  expect(result.search.keyRows.catalogHeaders).toBe(1);
  console.log('note-catalog-query-metrics',JSON.stringify(result));
});

for (const legacyVersion of [1,2]) test(`production browser database upgrades v${legacyVersion} metadata without loading all bodies and maintains canonical CAS`, async ({page}) => {
  test.setTimeout(120_000);
  execFileSync(process.execPath,[projectPath("scripts/prepare-note-query.mjs")],{stdio:'pipe'});
  await page.goto('/@vite/client');
  const result = await page.evaluate(async ({coreFile,legacyVersion}) => {
    const gateway = '/src/core/features/notes/gateways/';
    const {openIndexedDatabase,noteStores} = await import(gateway + 'browserNotesDatabase.ts');
    const {createCatalogStores} = await import(gateway + 'browserNoteCatalogStore.ts');
    const core = await import('/@fs/' + coreFile);
    const name = 'note-catalog-owner-probe-' + crypto.randomUUID();
    const assert = (value,message) => {if(!value)throw Error(message);};
    const complete = tx => new Promise((resolve,reject) => {tx.oncomplete=resolve;tx.onabort=tx.onerror=()=>reject(tx.error);});
    const pages=[], documents=[];
    for(let index=0;index<102;index++){
      const id=core.notePageId('n'+String(index).padStart(3,'0'));
      pages.push({id,title:'研究'+index,position:index,status:'active',isFavorite:false,revision:0,schemaVersion:1,projectIds:[],tagIds:[],createdAt:'2026-10-23T00:00:00Z',updatedAt:new Date(Date.UTC(2026,9,23)+index*1000).toISOString()});
      documents.push(core.createNoteDocumentFromText(id,index===73?'needle73 正文':index===0?'阅读正文首行':'普通正文 '+index,()=>core.noteBlockId(crypto.randomUUID())));
    }
    const old = await new Promise((resolve,reject) => {
      const request=indexedDB.open(name,legacyVersion);
      request.onupgradeneeded=()=>{
        for(const store of [...noteStores,'version'])request.result.createObjectStore(store);
        if(legacyVersion===2){request.transaction.objectStore('sources').createIndex('pageId','pageId');request.transaction.objectStore('links').createIndex('sourcePageId','sourcePageId');createCatalogStores(request.result);}
      };
      request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);
    });
    const initial=old.transaction([...noteStores,'version'],'readwrite'), initialDone=complete(initial);
    pages.forEach((page,index)=>{initial.objectStore('pages').put(page,page.id);initial.objectStore('documents').put(documents[index],page.id);});
    initial.objectStore('version').put(7,'current');
    initial.objectStore('operations').put({owner:'unchanged'},'opaque-hbs-record');
    initial.objectStore('sources').put({id:'reading-source',pageId:pages[0].id,noteType:'highlight',color:'yellow',source:{paperId:'paper',paperTitle:'来源',attachmentId:'attachment',contentSha256:'',pageIndex:0,rects:[],quoteExact:'摘录标题',quotePrefix:'',quoteSuffix:'',state:'exact'}},'reading-source');
    await initialDone;
    if(legacyVersion===2){
      const {applyBrowserNoteCatalogChanges}=await import(gateway+'browserNoteCatalogWrites.ts');
      const {loadBrowserNoteCatalogRuntime}=await import(gateway+'browserNoteCatalogRuntime.ts');
      const {catalogStores,catalogRequest,catalogTransactionDone}=await import(gateway+'browserNoteCatalogStore.ts');
      const runtime=await loadBrowserNoteCatalogRuntime();
      const projection=old.transaction([...catalogStores,'pages','sources','links','navigation'],'readwrite'),projected=catalogTransactionDone(projection);
      await applyBrowserNoteCatalogChanges(projection,pages.map(page=>({store:'pages',key:page.id,value:page})),runtime);
      const state=await catalogRequest(projection.objectStore('catalogState').get('current'));
      projection.objectStore('catalogState').put({...state,headersReady:true},'current');
      await projected;
    }
    old.close();
    let first,second,reopened;
    let documentReads=0;
    const original=IDBObjectStore.prototype.get;
    IDBObjectStore.prototype.get=function(...args){if(this.transaction.db.name===name&&this.name==='documents')documentReads++;return original.apply(this,args);};
    try {
      first=await openIndexedDatabase(name);
      const query={category:'all',view:'recent',search:'',limit:20};
      const recent=await first.queryCatalog(query);
      assert(recent.items.length===20&&recent.items[0].title==='研究101','upgraded recent order');
      const firstBodyReads=documentReads;assert(firstBodyReads===0,'metadata initialization must not load bodies');
      const canonicalBefore=await first.read({operations:['opaque-hbs-record']});
      assert(canonicalBefore.version===7&&canonicalBefore.rows.operations['opaque-hbs-record'].owner==='unchanged','derived initialization does not change canonical version/HBS');
      const renamed={...pages[73],title:'新题',revision:1};
      documentReads=0;await first.commit(7,[{store:'pages',key:renamed.id,value:renamed}]);
      assert(documentReads===0,'metadata commit reads no canonical bodies');
      let stale=false;try{await first.commit(7,[{store:'pages',key:renamed.id,value:{...renamed,title:'stale'}}]);}catch(error){stale=error.code==='revision_conflict';}
      assert(stale,'original canonical CAS retained');
      documentReads=0;
      const reading=await first.queryCatalog({...query,category:'dependent'});
      const readingTitleBodyReads=documentReads;
      assert(reading.items.length===1&&reading.items[0].title==='阅读正文首行'&&readingTitleBodyReads===1,'only visible legacy reading title is derived');
      documentReads=0;
      await first.queryCatalog({...query,sort:{field:'title',direction:'ascending'}});
      const titleSortBodyReads=documentReads;
      assert(titleSortBodyReads===0,'title ordering must not build the ordinary-body search index');
      documentReads=0;
      const search=await first.queryCatalog({...query,search:'needle73'});
      const initialSearchBodyReads=documentReads;
      assert(search.items.length===1&&search.items[0].title==='新题'&&initialSearchBodyReads===101,'on-demand resumable search backfill skips cached reading body');
      documentReads=0;
      await first.queryCatalog({...query,search:'needle73'});
      assert(documentReads===0,'ready persistent search reads no canonical bodies');
      second=await openIndexedDatabase(name);
      const updated={...renamed,revision:2};
      documentReads=0;
      await second.commit(8,[{store:'pages',key:updated.id,value:updated},{store:'documents',key:updated.id,value:core.createNoteDocumentFromText(updated.id,'nextneedle',()=>core.noteBlockId(crypto.randomUUID()))}]);
      assert(documentReads===0,'save uses incoming body rather than rereading canonical body');
      assert((await first.queryCatalog({...query,search:'needle73'})).items.length===0,'removed term not returned');
      assert((await first.queryCatalog({...query,search:'nextneedle'})).items.length===1,'second connection incremental text visible');
      first.close();first=undefined;second.close();second=undefined;
      reopened=await openIndexedDatabase(name);documentReads=0;
      await reopened.queryCatalog(query);assert(documentReads===0,'reopen reuses persistent catalog');
      const final=await reopened.read({operations:['opaque-hbs-record']});
      assert(final.version===9&&final.rows.operations['opaque-hbs-record'].owner==='unchanged','only two authorized commits change canonical version');
      return {firstBodyReads,readingTitleBodyReads,titleSortBodyReads,initialSearchBodyReads,staleCAS:stale,canonicalVersion:final.version};
    } finally {
      first?.close();second?.close();reopened?.close();IDBObjectStore.prototype.get=original;
      await new Promise((resolve,reject)=>{const request=indexedDB.deleteDatabase(name);request.onsuccess=resolve;request.onerror=()=>reject(request.error);request.onblocked=()=>reject(Error('owner probe delete blocked'));});
    }
  },{coreFile:publicPackagePath("@research-assistant/core/notes"),legacyVersion});
  expect(result).toEqual({firstBodyReads:0,readingTitleBodyReads:1,titleSortBodyReads:0,initialSearchBodyReads:101,staleCAS:true,canonicalVersion:9});
  console.log('note-catalog-owner-metrics',JSON.stringify(result));
});

test('a late reading source invalidates only missing reading-title progress', async ({page}) => {
  execFileSync(process.execPath,[projectPath("scripts/prepare-note-query.mjs")],{stdio:'pipe'});
  await page.goto('/@vite/client');
  const result = await page.evaluate(async coreFile => {
    const gateway='/src/core/features/notes/gateways/';
    const {openIndexedDatabase}=await import(gateway+'browserNotesDatabase.ts');
    const core=await import('/@fs/'+coreFile);
    const name='note-catalog-late-source-'+crypto.randomUUID(),db=await openIndexedDatabase(name);
    const id=core.notePageId('pending-reading');
    const saved={id,title:'原始标题',position:0,status:'active',isFavorite:false,revision:0,schemaVersion:1,projectIds:[],tagIds:[],createdAt:'2026-10-23T00:00:00Z',updatedAt:'2026-10-23T00:00:00Z'};
    let reads=0;const get=IDBObjectStore.prototype.get;
    IDBObjectStore.prototype.get=function(...args){if(this.transaction.db.name===name&&this.name==='documents')reads++;return get.apply(this,args);};
    try{
      await db.commit(0,[{store:'pages',key:id,value:saved},{store:'documents',key:id,value:core.createNoteDocumentFromText(id,'真实阅读标题',()=>core.noteBlockId('block'))}]);
      const query={category:'all',view:'recent',search:'',sort:{field:'title',direction:'ascending'},limit:20};
      await db.queryCatalog(query);if(reads!==0)throw Error('ordinary title sort read a body');
      let invalid=false;
      try{await db.queryCatalog({...query,search:'中'.repeat(171)});}catch(error){invalid=error.code==='invalid_input'&&String(error.message).includes('note_catalog_search_invalid');}
      if(!invalid||reads!==0)throw Error('invalid query scheduled a body backfill or lost its diagnostic');
      await db.commit(1,[{store:'sources',key:'reading',value:{id:'reading',pageId:id,noteType:'highlight',color:'yellow',source:{paperId:'paper',paperTitle:'来源',attachmentId:'attachment',contentSha256:'',pageIndex:0,rects:[],quoteExact:'摘录回退标题',quotePrefix:'',quoteSuffix:'',state:'exact'}}}]);
      const page=await db.queryCatalog(query);
      if(reads!==1||page.items[0].title!=='真实阅读标题')throw Error('late reading source reused stale title readiness');
      return{reads,title:page.items[0].title};
    }finally{
      db.close();IDBObjectStore.prototype.get=get;
      await new Promise((resolve,reject)=>{const request=indexedDB.deleteDatabase(name);request.onsuccess=resolve;request.onerror=()=>reject(request.error);});
    }
  },publicPackagePath("@research-assistant/core/notes"));
  expect(result).toEqual({reads:1,title:'真实阅读标题'});
});
