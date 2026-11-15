import { Dialog } from "../../../components/overlay/OverlayDialog";
import { DialogHeading } from "@ariakit/react";
import { X } from "lucide-react";
import {useEffect,useState,type RefObject} from "react";
import {catalogErrorMessage} from "../noteCatalogError";
import {
  noteBlockId,
  notePageId,
  type NoteBlockId,
  type NoteLink,
  type NotePage,
  type NotePageId,
} from "@research-assistant/core/notes";

interface NoteRelationsProps {
  readonly finalFocus: RefObject<HTMLButtonElement | null>;
  readonly onClose: () => void;
  readonly outgoing?: readonly NoteLink[];
  readonly backlinks: readonly NoteLink[];
  readonly pages: readonly NotePage[];
  readonly readPageMetadata?: ((ids:readonly NotePageId[])=>Promise<readonly NotePage[]>)|undefined;
  readonly onOpenPage: (id: NotePageId) => void;
  readonly onOpenReading: (id: NoteBlockId) => void;
}

export function NoteRelations({ outgoing, backlinks, pages, onOpenPage, onOpenReading, onClose, finalFocus,readPageMetadata }: NoteRelationsProps): React.JSX.Element {
  const [loaded,setLoaded]=useState<readonly NotePage[]>([]),[loading,setLoading]=useState(false),[error,setError]=useState<string>(),[retry,setRetry]=useState(0);
  const known=new Map([...loaded,...pages].map(page=>[page.id,page]));
  const missing=[...new Set(backlinks.map(link=>link.sourcePageId))].filter(id=>!known.has(id));
  const signature=JSON.stringify(missing);
  useEffect(()=>{
    if(!readPageMetadata||!missing.length)return;
    let current=true;setLoading(true);setError(undefined);
    void(async()=>{
      const result:NotePage[]=[];
      for(let offset=0;current&&offset<missing.length;offset+=200)result.push(...await readPageMetadata(missing.slice(offset,offset+200)));
      if(current){setLoading(false);setLoaded(previous=>[...previous,...result]);}
    })().catch(reason=>{if(current){setLoading(false);setError(catalogErrorMessage(reason));}});
    return()=>{current=false;};
  },[readPageMetadata,signature,retry]);
  return <Dialog open portal modal={false} hideOnInteractOutside={false} finalFocus={finalFocus} onClose={onClose} className="note-relations" aria-label="出链与反链">
    <header><DialogHeading render={<h2 />}>出链与反链</DialogHeading><button aria-label="关闭出链与反链" onClick={onClose} type="button"><X aria-hidden="true" size={16} /></button></header>
    {loading&&<p role="status">正在读取引用页面…</p>}
    {error&&<p role="alert">{error}<button type="button" onClick={()=>setRetry(value=>value+1)}>重试</button></p>}
    {outgoing !== undefined && <section><strong>出链</strong>{outgoing.length === 0 ? <small>尚未引用其他研究对象</small> : outgoing.map((link) => <button className={link.isBroken ? "broken" : ""} key={link.id} onClick={() => {
      if (link.target.kind === "note_page") onOpenPage(notePageId(link.target.id));
      else if (link.target.kind === "reading_note") onOpenReading(noteBlockId(link.target.id));
      else { onClose(); window.dispatchEvent(new CustomEvent("research-assistant:open-research-target", { detail: link.target })); }
    }} type="button"><span>{link.target.labelSnapshot || link.target.id}</span><small>{link.relation}{link.isBroken ? " · 目标不可用" : ""}</small></button>)}</section>}
    <section><strong>反链</strong>{backlinks.length === 0 ? <small>尚无页面引用这条笔记</small> : backlinks.map((link) => <button key={link.id} onClick={() => onOpenPage(link.sourcePageId)} type="button"><span>{known.get(link.sourcePageId)?.title ?? link.sourcePageId}</span><small>{link.relation}</small></button>)}</section>
  </Dialog>;
}
