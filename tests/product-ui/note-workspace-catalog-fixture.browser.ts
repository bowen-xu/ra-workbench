import type {Page} from "@playwright/test";

/** Synthetic persistence only. The acceptance page is the actual App, gateways,
 * note workspace, editor, selector and overlay tree after a real reload. */
export async function seedNoteWorkspaceCatalog(page:Page,paths:{core:string;data:string;writing:string}){
  await page.goto('/?preview=app');await page.getByRole('button',{name:'笔记',exact:true}).waitFor();
  return page.evaluate(async paths=>{
    const core=await import('/@fs/'+paths.core),data=await import('/@fs/'+paths.data),writing=await import('/@fs/'+paths.writing);
    const {getAppStorage,appStorageEventKey}=await import('/ra-workbench/apps/web/src/core/runtime/appStorage.ts'),{flushStorage}=await import('/ra-workbench/apps/web/src/core/runtime/storagePersistence.ts');
    const {getWritingGateway}=await import('/plugins/ra-writing-projects/src/features/writing/gateway/index.ts'),{writingFileKey}=await import('/ra-workbench/apps/web/src/core/public/writingStorage.ts');
    const {emptyWritingCatalogState}=await import('/ra-workbench/apps/web/src/core/features/writing-storage/writingCatalogState.ts');
    const {loadBrowserNoteCatalogRuntime}=await import('/ra-workbench/apps/web/src/core/features/notes/gateways/browserNoteCatalogRuntime.ts');
    const {projectWritingCatalogContent,projectWritingCatalogHeader,writingCatalogId}=await import('/ra-workbench/apps/web/src/core/features/writing-storage/writingCatalogProjection.ts');
    const {browserCatalogStores:catalogStores,catalogTransactionDone,catalogRequest}=await import('/ra-workbench/apps/web/src/core/features/notes/gateways/browserNoteCatalogStore.ts');
    const runtime=await loadBrowserNoteCatalogRuntime(),storage=getAppStorage(),gateway=getWritingGateway();
    const project=(await gateway.listManuscripts()).find(project=>project.status==='active');
    if(!project)throw Error('synthetic project missing');
    const archived=await gateway.archiveManuscript((await gateway.createManuscript({title:'归档查询夹具'})).id);
    await flushStorage(storage);
    const names={notes:'research-assistant.notes.'+storage.getItem('research-assistant.notes.v1.indexeddb'),writing:'research-assistant.preview.'+encodeURIComponent(appStorageEventKey(''))};
    const open=(name:string)=>new Promise<any>((resolve,reject)=>{const request=indexedDB.open(name);request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);});
    const body=(i:number)=>`${i%2?'阅读':'普通'}正文 ${i}\n${i===1737?'unique-needle-1737':i===2237?'archive-needle-2237':'合成正文'}\n用于检查列表、编辑器保留和索引增量。`;
    const base=Date.now()-4_800_000;
    const noteEntries=Array.from({length:2400},(_,index)=>{
      const id=core.notePageId('catalog-note-'+String(index).padStart(4,'0')),reading=index%2===1,timestamp=new Date(base+index*2000).toISOString();
      const page={id,title:`${reading?'阅读':'普通'}笔记 ${index}`,schemaVersion:1,revision:1,position:index,status:index<2000?'active':'archived',isFavorite:!reading&&index%1000===0,projectIds:[],tagIds:[],createdAt:timestamp,updatedAt:timestamp};
      let block=0;const document=core.createNoteDocumentFromText(id,body(index),()=>core.noteBlockId(`catalog-block-${index}-${block++}`));
      const source=reading?{id:core.noteBlockId(id),pageId:id,noteType:'highlight',color:'yellow',source:{paperId:'10000000-0000-4000-8000-000000000001',paperTitle:'合成文献',attachmentId:'10000000-0000-4000-8000-000000000002',contentSha256:'',pageIndex:0,rects:[],quoteExact:'模拟摘录',quotePrefix:'',quoteSuffix:'',state:'exact'}}:undefined;
      const text=core.noteDocumentPlainText(document),foldedText=runtime.fold(text),grams=[...data.noteSearchGrams(foldedText)];
      const header=data.projectNoteCatalogHeader({page,...(source?{source}:{}),links:[],bodyFirstLine:source?body(index).split('\n')[0]:'',ordinal:index+1,...(page.isFavorite?{favoritePosition:index/1000}:{})},runtime);
      return{page,document,source,header,projection:{text,foldedText,grams}};
    });
    const writingEntries=await Promise.all(Array.from({length:2400},async(_,index)=>{
      const owner=index<2000?project:archived,path=`catalog/${String(index).padStart(4,'0')}.ranote`,content=JSON.stringify(writing.createWritingNote('catalog-writing-'+index,'catalog-wb-'+index,body(index)));
      const sha256=[...new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(content)))].map(value=>value.toString(16).padStart(2,'0')).join('');
      const file={manuscriptId:owner.id,relativePath:path,kind:'other',sha256,sizeBytes:new TextEncoder().encode(content).length,modifiedAt:new Date(base+index*2000+1000).toISOString()};
      const id=writingCatalogId(owner.id,'catalog-writing-'+index),header=projectWritingCatalogHeader(file,owner,id,index+1,runtime),text=projectWritingCatalogContent(content).text,foldedText=runtime.fold(text),grams=[...data.noteSearchGrams(foldedText)];
      return{key:writingFileKey(owner.id,path),file,body:{relativePath:path,content,sha256},header,projection:{text,foldedText,grams},work:{manuscriptId:owner.id,path,sha256,status:owner.status,state:'ready',id}};
    }));
    const populate=(tx:IDBTransaction,entries:any[])=>{
      const postings=new Map<string,number[]>();
      for(const entry of entries){
        tx.objectStore('catalogHeaders').put(entry.header);tx.objectStore('catalogBodies').put(entry.projection,entry.header.item.id);
        for(const [component,values]of [[entry.header.ordinal*2,entry.projection.grams],[entry.header.ordinal*2+1,data.noteSearchGrams(entry.header.metadataText)]])for(const gram of values){if(!postings.has(gram))postings.set(gram,[]);postings.get(gram)!.push(component);}
      }
      for(const [gram,values]of postings){tx.objectStore('catalogPostingRows').put({schema:1,gram,count:values.length,value:data.encodeNoteSearchPosting(Uint32Array.from(values))});}
    };
    const notes=await open(names.notes),writer=await open(names.writing);
    try{
      const canonical=['pages','documents','sources','links','revisions','navigation','operations','conflicts'];
      const tx=notes.transaction([...canonical,...catalogStores,'version'],'readwrite'),done=catalogTransactionDone(tx);
      for(const name of [...canonical,...catalogStores])tx.objectStore(name).clear();
      for(const row of noteEntries){tx.objectStore('pages').put(row.page,row.page.id);tx.objectStore('documents').put(row.document,row.page.id);if(row.source){tx.objectStore('sources').put(row.source,row.source.id);const id=`reading-origin:${row.source.id}`;tx.objectStore('links').put({id,sourcePageId:row.page.id,target:{kind:'paper',id:row.source.source.paperId,labelSnapshot:row.source.source.paperTitle,quoteSnapshot:row.source.source.quoteExact},relation:'derived_from',isBroken:false,createdAt:row.page.createdAt},id);}}
      tx.objectStore('navigation').put(noteEntries.filter(row=>row.page.isFavorite).map(row=>row.page.id),'favoriteOrder');
      populate(tx,noteEntries);
      tx.objectStore('catalogState').put({generation:10,nextOrdinal:2401,collationVersion:runtime.collationVersion,normalizationVersion:runtime.normalizationVersion,gramVersion:data.NOTE_SEARCH_GRAM_VERSION,headersReady:true,searchReady:true,readingTitlesReady:true,searchMetadataReady:true},'current');
      const version=tx.objectStore('version').get('current');version.onsuccess=()=>tx.objectStore('version').put((version.result??0)+1,'current');await done;
      const stores=[...catalogStores,'writingCatalogFiles','writingFiles','writingBodies','metadata'];
      const write=writer.transaction(stores,'readwrite'),written=catalogTransactionDone(write);
      for(const name of [...catalogStores,'writingCatalogFiles'])write.objectStore(name).clear();
      const old=await catalogRequest(write.objectStore('writingFiles').getAll());
      for(const file of old)if(file.relativePath.toLowerCase().endsWith('.ranote')){const key=writingFileKey(file.manuscriptId,file.relativePath);write.objectStore('writingFiles').delete(key);write.objectStore('writingBodies').delete(key);}
      for(const row of writingEntries){write.objectStore('writingFiles').put(row.file,row.key);write.objectStore('writingBodies').put(row.body,row.key);write.objectStore('writingCatalogFiles').put(row.work,row.key);}
      populate(write,writingEntries);
      const state=emptyWritingCatalogState();write.objectStore('catalogState').put({...state,generation:10,nextOrdinal:2401,collationVersion:runtime.collationVersion,normalizationVersion:runtime.normalizationVersion,writing:{...state.writing,counts:{active:{ready:2000,pending:0,error:0},archived:{ready:400,pending:0,error:0}}}},'current');
      const revision=write.objectStore('metadata').get('writingRevision');revision.onsuccess=()=>write.objectStore('metadata').put((revision.result??0)+1,'writingRevision');await written;
    }finally{notes.close();writer.close();}
    return{names,ordinaryId:'catalog-note-1998',ordinaryTitle:'普通笔记 1998',rows:4800};
  },paths);
}
