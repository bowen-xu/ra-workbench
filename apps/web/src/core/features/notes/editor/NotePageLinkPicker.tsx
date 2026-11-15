import {useEffect,useRef,useState} from "react";
import {X} from "lucide-react";
import type {NoteTargetQuery} from "@research-assistant/data/notes";
import {Dialog} from "../../../components/overlay/OverlayDialog";
import {PageLinkBlock} from "../links/PageLinkBlock";
import {useNotesClient,useNotesVersion} from "../NotesClient";
import {useNoteCatalogTargets} from "../workspace/useNoteCatalogTargets";
import {catalogErrorMessage} from "../noteCatalogError";

export function NotePageLinkPicker({pages,onSelect,onClose,queryTargets,excludePageId}:{
  readonly pages:readonly {readonly id:string;readonly title:string}[];
  readonly onSelect:(page:{readonly id:string;readonly title:string})=>void;readonly onClose:()=>void;
  readonly queryTargets?:NoteTargetQuery|undefined;readonly excludePageId?:string;
}):React.JSX.Element{
  const [query,setQuery]=useState(""),[available,setAvailable]=useState(pages),[loading,setLoading]=useState(false),[error,setError]=useState<string>();
  const [cursor,setCursor]=useState<string>(),[retry,setRetry]=useState(0);
  const client=useNotesClient(),version=useNotesVersion(),fallback=useNoteCatalogTargets(client,version);
  const provider=queryTargets??(pages.length?undefined:fallback),generation=useRef(0),staticPages=provider?undefined:pages;
  const run=async(search:string,position:string|undefined,request:number)=>{
    if(!provider)return;setLoading(true);setError(undefined);
    try{
      const page=await provider(search,position);
      if(request!==generation.current)return;
      const selected=page.items.flatMap(item=>item.target.kind==="writing"||item.target.pageId===excludePageId?[]:[{id:item.target.pageId,title:item.title}]);
      setAvailable(previous=>position?[...previous,...selected]:selected);setCursor(page.nextCursor);
    }catch(reason){if(request===generation.current)setError(catalogErrorMessage(reason));}
    finally{if(request===generation.current)setLoading(false);}
  };
  useEffect(()=>{
    if(!provider){setAvailable(staticPages??[]);setLoading(false);return;}
    const request=++generation.current;setAvailable([]);setCursor(undefined);setError(undefined);setLoading(true);
    const timer=window.setTimeout(()=>{void run(query,undefined,request);},query.trim()?180:0);
    return()=>{generation.current++;window.clearTimeout(timer);};
  },[provider,query,staticPages,retry,excludePageId]);
  const matches=provider?available:available.filter(page=>page.title.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
  return <Dialog open onClose={onClose} aria-label="链接到笔记" className="note-page-link-picker">
    <header><h2>链接到笔记</h2><button aria-label="关闭笔记选择" type="button" onClick={onClose}><X size={17}/></button></header>
    <input aria-label="查找要链接的笔记" placeholder="搜索笔记标题" value={query} onChange={event=>setQuery(event.target.value)}/>
    {matches.map(page=><PageLinkBlock key={page.id} title={page.title} onOpen={()=>onSelect(page)}/>)}
    {loading&&<p role="status">正在读取笔记…</p>}
    {error&&<p role="alert">{error}<button type="button" onClick={()=>setRetry(value=>value+1)}>重试</button></p>}
    {!loading&&!error&&!matches.length&&<p>没有找到可链接的笔记</p>}
    {cursor&&<button type="button" disabled={loading} onClick={()=>{void run(query,cursor,++generation.current);}}>加载更多笔记</button>}
  </Dialog>;
}
