import type {Page} from "@playwright/test";

export async function installNoteWorkspaceQueryMetrics(page:Page,names:{notes:string;writing:string}){
  await page.addInitScript(names=>{
    const make=()=>({rows:{},keys:{},requests:{},writes:{},documentIds:[],pending:0});
    const state={current:make(),reset(){if(this.current.pending)throw Error('cannot reset an unfinished query stage');this.current=make();}};
    window.__noteWorkspaceQueryMetrics=state;
    for(const [prototype,methods]of [[IDBObjectStore.prototype,['get','getAll','getAllKeys','getKey','count','openCursor','put','add','delete','clear']],[IDBIndex.prototype,['get','getAll','getAllKeys','getKey','count','openCursor','openKeyCursor']]])for(const method of methods){
      const original=prototype[method];if(!original)continue;
      prototype[method]=function(...args){
        const store=this instanceof IDBIndex?this.objectStore:this,db=store.transaction.db;
        const request=original.apply(this,args);if(db.name!==names.notes&&db.name!==names.writing)return request;
        const metric=state.current,source=db.name===names.notes?'notes':'writing',key=`${source}.${store.name}`;
        const writes=['put','add','delete','clear'].includes(method),cursor=method==='openCursor'||method==='openKeyCursor',keyOnly=['getKey','getAllKeys','openKeyCursor'].includes(method);
        const group=writes?metric.writes:metric.requests;group[key+'.'+method]=(group[key+'.'+method]??0)+1;
        metric.pending++;let finished=false;const settle=()=>{if(!finished){finished=true;metric.pending--;}};
        request.addEventListener('error',settle,{once:true});
        if(cursor){store.transaction.addEventListener('complete',settle,{once:true});store.transaction.addEventListener('abort',settle,{once:true});}
        request.addEventListener('success',()=>{
          const result=request.result;if(!cursor)settle();
          if(writes||method==='count'||result===undefined||result===null)return;
          const counts=keyOnly?metric.keys:metric.rows;counts[key]=(counts[key]??0)+(['getAll','getAllKeys'].includes(method)?result.length:1);
          if(!keyOnly&&source==='notes'&&store.name==='documents'&&metric.documentIds.length<64){
            for(const value of method==='getAll'?result:[cursor?result.value:result])if(!metric.documentIds.includes(value.pageId))metric.documentIds.push(value.pageId);
          }
        });
        return request;
      };
    }
  },names);
}
export async function resetNoteWorkspaceQueryMetrics(page:Page){await page.evaluate(()=>window.__noteWorkspaceQueryMetrics.reset());}
export async function readNoteWorkspaceQueryMetrics(page:Page){return page.evaluate(()=>structuredClone(window.__noteWorkspaceQueryMetrics.current));}
