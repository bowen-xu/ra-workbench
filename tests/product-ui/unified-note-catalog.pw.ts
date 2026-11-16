import {publicPackagePath} from "../ui/projectPaths";
import {test,expect} from "@playwright/test";

test("unified persisted sources merge bounded pages without dropping tails or reading canonical bodies",async ({page})=>{
  test.setTimeout(120_000); await page.goto("/@vite/client");
  const names = await page.evaluate(async dataFile=>{
    const api=await import("/@fs/"+dataFile), store=await import("/ra-workbench/apps/web/src/core/features/notes/gateways/browserNoteCatalogStore.ts");
    const {loadBrowserNoteCatalogRuntime}=await import("/ra-workbench/apps/web/src/core/features/notes/gateways/browserNoteCatalogRuntime.ts");
    const runtime=await loadBrowserNoteCatalogRuntime(), prefix="unified-note-catalog-"+crypto.randomUUID(), names=[prefix+"-pages",prefix+"-writing"];
    for(let source=0;source<2;source++){
      const db=await new Promise((resolve,reject)=>{
        const r=indexedDB.open(names[source],1);
        r.onupgradeneeded=()=>{
          store.createCatalogStores(r.result);
          const indexedPostings=r.result.createObjectStore("catalogPostingRows",{keyPath:"gram"});
          indexedPostings.createIndex("gramCount",["gram","count"],{unique:true});
          r.result.createObjectStore("documents");
        };
        r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);
      });
      try{
        const tx=db.transaction([...store.catalogStores,"documents"],"readwrite"), done=store.catalogTransactionDone(tx), postings=new Map();
        for(let index=0;index<2400;index++){
          const id=String(index).padStart(5,"0"), writing=source===1, dependent=writing||index%3===0;
          const item={id:writing?`writing:${JSON.stringify(["project",id])}`:`page:${id}`,title:`研究${index%73}`,snippet:"",category:dependent?"dependent":"independent",
            ...(writing?{owner:{kind:"manuscript",id:"project",label:`项目${index%7}`}}:dependent?{owner:{kind:"highlight",id,label:`文献${index%7}`}}:{sourceLabel:`来源${index%7}`}),
            updatedAt:new Date(Date.UTC(2026,9,24)+(index*2+source)*1000).toISOString(),status:index%11===0?"archived":"active",isFavorite:!dependent&&index%2===0,
            ...(!dependent&&index%2===0?{favoritePosition:2400-index}:{}),target:writing?{kind:"writing",manuscriptId:"project",path:id+".ranote"}:dependent?{kind:"reading",pageId:id,noteId:id}:{kind:"page",pageId:id}};
          const text=index===1737?"unique-needle-1737":"正文", foldedText=runtime.fold(text), grams=[...api.noteSearchGrams(foldedText)];
          const header=api.projectNoteCatalogItemHeader({item,metadataText:[item.title,item.owner?.label,item.sourceLabel].filter(Boolean).join(" "),ordinal:index+1},runtime);
          tx.objectStore("catalogHeaders").put(header);tx.objectStore("catalogBodies").put({text,foldedText,grams},item.id);tx.objectStore("documents").put({privateBody:"canonical sentinel"},item.id);
          for(const [component,values]of [[header.ordinal*2,grams],[header.ordinal*2+1,api.noteSearchGrams(header.metadataText)]])for(const gram of values){if(!postings.has(gram))postings.set(gram,[]);postings.get(gram).push(component);}
        }
        for(const [gram,values]of postings){tx.objectStore("catalogPostings").put(Uint32Array.from(values),gram);tx.objectStore("catalogCounts").put(values.length,gram);}
        tx.objectStore("catalogState").put({generation:1,nextOrdinal:2401,collationVersion:runtime.collationVersion,normalizationVersion:runtime.normalizationVersion,gramVersion:api.NOTE_SEARCH_GRAM_VERSION,searchReady:true,headersReady:true},"current");await done;
      }finally{db.close();}
    }
    return names;
  },publicPackagePath("@research-assistant/data/notes"));
  // Drop the seed page's Wasm/runtime module cache before the cold query.
  await page.reload();
  const result=await page.evaluate(async ({names,dataFile})=>{
    const api=await import("/@fs/"+dataFile),store=await import("/ra-workbench/apps/web/src/core/features/notes/gateways/browserNoteCatalogStore.ts");
    const {loadBrowserNoteCatalogRuntime}=await import("/ra-workbench/apps/web/src/core/features/notes/gateways/browserNoteCatalogRuntime.ts");
    const databases=await Promise.all(names.map(name=>new Promise((resolve,reject)=>{const r=indexedDB.open(name);r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);})));
    const reads={},originals=[],calls=[[],[]];let runtimeLoads=0;
    for(const [prototype,methods]of [[IDBObjectStore.prototype,["get","getAll","openCursor"]],[IDBIndex.prototype,["get","getAll","getKey","openCursor","openKeyCursor"]]])for(const method of methods){
      const original=prototype[method];originals.push([prototype,method,original]);prototype[method]=function(...args){const object=this instanceof IDBIndex?this.objectStore:this,request=original.apply(this,args);
        if(names.includes(object.transaction.db.name))request.addEventListener("success",()=>{if(request.result!==undefined&&request.result!==null){const key=object.name;reads[key]=(reads[key]??0)+(method==="getAll"?request.result.length:1);}});return request;};
    }
    const reset=()=>{for(const key of Object.keys(reads))delete reads[key];};
    const assert=(condition,message)=>{if(!condition)throw Error(message);};
    const source=async(index,query)=>{calls[index].push({limit:query.limit,cursor:!!query.cursor,search:query.search});return store.queryBrowserNoteCatalog(databases[index],query,query.search.trim()?await loadBrowserNoteCatalogRuntime():undefined);};
    const catalog=new api.UnifiedNoteCatalog({queryNotePageCatalog:q=>source(0,q)},{queryWritingNoteCatalog:async q=>({...await source(1,q),pendingCount:0,errorCount:0,issues:[]})},async()=>{runtimeLoads++;return loadBrowserNoteCatalogRuntime();});
    const q={category:"all",view:"recent",search:"",limit:20};
    try{
      const first=await catalog.query(q),cold={...reads};assert(first.items.length===20&&cold.catalogHeaders===46&&!cold.documents&&!cold.catalogBodies,"bounded first merged page");
      reset();const second=await catalog.query({...q,cursor:first.nextCursor}),continuation={...reads};assert(second.items.length===20&&continuation.catalogHeaders<=52&&!continuation.documents&&!continuation.catalogBodies,"reuse unconsumed source tails "+JSON.stringify(continuation));
      assert(new Set([...first.items,...second.items].map(item=>item.id)).size===40,"no duplicated tails");assert(runtimeLoads===0,"cold date merge does not load collation");
      const coldWasm=performance.getEntriesByType("resource").filter(r=>r.name.includes("note-query")&&r.name.endsWith(".wasm")).length;
      reset();const searched=await catalog.query({...q,search:"unique-needle-1737"}),search={...reads};assert(searched.items.length===2&&!search.documents&&search.catalogBodies===2,"two persistent search projections only");
      const runtime=await loadBrowserNoteCatalogRuntime(), compareBytes=(a,b)=>{for(let i=0;i<Math.min(a.length,b.length);i++)if(a[i]!==b[i])return a[i]-b[i];return a.length-b.length;};
      let cases=0;
      for(const field of ["title","category","source","updatedAt"])for(const direction of ["ascending","descending"])for(const view of ["recent","archive"])for(const category of ["all","dependent"]){
        const query={...q,view,category,sort:{field,direction}},a=await catalog.query(query),b=await catalog.query({...query,cursor:a.nextCursor}),items=[...a.items,...b.items];
        const key=item=>field==="updatedAt"?Date.parse(item.updatedAt):runtime.sortKey(field==="title"?item.title:field==="category"?item.category==="independent"?"独立笔记":"附属笔记":item.owner?.label??item.sourceLabel??"");
        for(let i=1;i<items.length;i++){const left=key(items[i-1]),right=key(items[i]),difference=field==="updatedAt"?left-right:compareBytes(left,right),tie=compareBytes(new TextEncoder().encode(items[i-1].id),new TextEncoder().encode(items[i].id));assert((direction==="ascending"?difference:-difference)<0||difference===0&&tie<0,"merged shared-kernel sort "+JSON.stringify(query));}cases++;
      }
      reset();const unchanged=await catalog.query(q),tx=databases[1].transaction("catalogState","readwrite"),done=store.catalogTransactionDone(tx),state=await store.catalogRequest(tx.objectStore("catalogState").get("current"));tx.objectStore("catalogState").put({...state,generation:2},"current");await done;
      let stale=false;try{await catalog.query({...q,cursor:unchanged.nextCursor});}catch(error){stale=error.code==="revision_conflict";}assert(stale,"source change invalidates merged cursor");
      return {rows:4800,cold,continuation,search,cases,coldWasm,stale};
    }finally{
      for(const [prototype,method,original]of originals)prototype[method]=original;for(const db of databases)db.close();
      await Promise.all(names.map(name=>new Promise((resolve,reject)=>{const r=indexedDB.deleteDatabase(name);r.onsuccess=resolve;r.onerror=()=>reject(r.error);})));
    }
  },{names,dataFile:publicPackagePath("@research-assistant/data/notes")});
  expect(result).toMatchObject({rows:4800,cases:32,coldWasm:0,stale:true});console.log("unified-note-catalog",JSON.stringify(result));
});
