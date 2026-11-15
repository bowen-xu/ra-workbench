import {createContext,useCallback,useContext,useId,useLayoutEffect,useMemo,useRef,useState,type ReactNode} from "react";
import {Undo2} from "lucide-react";
import {ControlHint} from "../../components/ControlHint";
import {AgentChangeSummary,type AgentChangeSummaryItem} from "./AgentChangeSummary";
import {notify} from "../../services/notifications";

export interface AgentChangeUndo {
 readonly disabled:boolean;
 readonly busy?:boolean;
 readonly hint?:string|undefined;
 readonly title?:string|undefined;
 readonly label?:string;
 /** Existing resource owner reports confirmation; false stops group dispatch. */
 readonly run:()=>Promise<boolean>;
}
interface Contribution {
 readonly ariaLabel:string;
 /** Original change owner, independent of React mounts and Undo executions. */
 readonly ownerIdentity?:string;
 readonly items:readonly AgentChangeSummaryItem[];
 readonly stats?:{readonly additions:number;readonly deletions:number};
 readonly undo?:AgentChangeUndo|undefined;
 readonly children?:ReactNode;
}
interface Registry {
 readonly update:(id:string,value:Contribution)=>void;
 readonly remove:(id:string)=>void;
}
const SummaryContext=createContext<Registry|null>(null);
const undoIdentity=(registration:string,part:Contribution)=>JSON.stringify(part.ownerIdentity===undefined
 ?["registration",registration]:["change-owner",part.ownerIdentity]);

/** The owning turn alone renders the card. Resource adapters retain their own
 * snapshots, decisions and recovery; nested scopes cannot create another card. */
export function AgentTurnChangeSummary({children}:{readonly children:ReactNode}){
 const parent=useContext(SummaryContext);
 return parent?<>{children}</>:<TurnSummary>{children}</TurnSummary>;
}
function TurnSummary({children}:{readonly children:ReactNode}){
 const [parts,setParts]=useState<ReadonlyMap<string,Contribution>>(new Map());
 const [completed,setCompleted]=useState<ReadonlySet<string>>(new Set());
 const [busy,setBusy]=useState(false);
 const pending=useRef(false);
 const update=useCallback((id:string,value:Contribution)=>setParts(previous=>new Map(previous).set(id,value)),[]);
 const remove=useCallback((id:string)=>setParts(previous=>{const next=new Map(previous);next.delete(id);return next;}),[]);
 const registry=useMemo(()=>({update,remove}),[update,remove]);
 const entries=[...parts].filter(([,part])=>part.items.length>0);
 const unique=new Map<string,{owner:string;item:AgentChangeSummaryItem}>();
 for(const [owner,part] of entries)for(const item of part.items)unique.set(item.identity??`${owner}/${item.id}`,{owner,item});
 const owned=[...unique.values()],items=owned.map(({owner,item})=>({...item,id:`${owner}/${item.id}`}));
 const owners=new Set(owned.map(value=>value.owner));
 const undoOwners=new Map<string,Contribution[]>();
 for(const [id,part] of entries)if(owners.has(id)){
  const identity=undoIdentity(id,part),group=undoOwners.get(identity);
  if(group)group.push(part);else undoOwners.set(identity,[part]);
 }
 const remaining=[...undoOwners].filter(([identity])=>!completed.has(identity));
 const remainingParts=remaining.flatMap(([,group])=>group);
 const completedCurrent=[...undoOwners.keys()].some(identity=>completed.has(identity));
 const undoVisible=entries.some(([,part])=>part.undo);
 const disabled=busy||remaining.length===0||remainingParts.some(part=>!part.undo||part.undo.disabled||part.undo.busy);
 const hint=remainingParts.find(part=>part.undo?.disabled)?.undo?.hint
  ??(remainingParts.some(part=>!part.undo)?"部分修改当前不能整组撤销，请在原位置审阅。":entries.length===1?entries[0]?.[1].undo?.hint:undefined);
 const stats=items.length>0&&items.every(item=>item.stats)?items.reduce((sum,item)=>({additions:sum.additions+item.stats!.additions,deletions:sum.deletions+item.stats!.deletions}),{additions:0,deletions:0})
  :entries.length===1?entries[0]?.[1].stats:undefined;
 const undo=async()=>{
  if(pending.current||disabled)return;
  pending.current=true;setBusy(true);
  try{
   for(const [identity,group] of remaining){
    const action=group[0]?.undo;
    if(!action||!await action.run())break;
    setCompleted(previous=>new Set([...previous,identity]));
   }
  }catch(reason){notify.error(`撤销失败：${reason instanceof Error?reason.message:String(reason)}`);}
  finally{pending.current=false;setBusy(false);}
 };
 const undoDescription=remainingParts.find(part=>part.undo?.title)?.undo?.title;
 const button=undoVisible?<ControlHint content={hint ?? undoDescription} wrap><button aria-label="Undo" aria-description={undoDescription} className="writing-agent-inline-diff-undo" type="button" disabled={disabled} onClick={()=>void undo()}>
  <Undo2 aria-hidden="true" size={14}/><span>{busy||entries.some(([,part])=>part.undo?.busy)?"Undo…":entries.length===1?entries[0]?.[1].undo?.label??"Undo":"Undo"}</span>
 </button></ControlHint>:undefined;
 return <SummaryContext.Provider value={registry}>{children}{items.length>0&&<section aria-label="补丁结果"><AgentChangeSummary
  title={`Edited ${items.length} files`} ariaLabel={entries.length===1?entries[0]![1].ariaLabel:"本轮变更审阅"}
  items={items} {...stats?{stats}:{}} headerActions={button}>
  {completedCurrent&&remaining.length>0&&<p role="status">部分修改已撤销，其余修改保留。</p>}
  {entries.map(([id,part])=><div key={id}>{part.children}</div>)}
 </AgentChangeSummary></section>}</SummaryContext.Provider>;
}
function RegisteredContribution(props:Contribution){
 const registry=useContext(SummaryContext)!,id=useId();
 useLayoutEffect(()=>{registry.update(id,props);},[registry,id,props]);
 useLayoutEffect(()=>()=>registry.remove(id),[registry,id]);
 return null;
}
/** Standalone previews use the same owner; adapters only contribute entries. */
export function AgentChangeContribution(props:Contribution){
 return <AgentTurnChangeSummary><RegisteredContribution {...props}/></AgentTurnChangeSummary>;
}
