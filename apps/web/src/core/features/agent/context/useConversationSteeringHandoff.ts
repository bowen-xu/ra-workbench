import {useEffect,useRef,useState} from "react";
import type {AgentConversationProjection,AgentQueueEditScope,AgentQueueState} from "@research-assistant/core/agent";
import {AgentSteeringHandoffController,type AgentSteeringHandoffState,type AgentSteeringHandoffRegistry} from "@research-assistant/data/agent";
import type {AgentContextGateway} from "../../../services/agentContextGateway";
import {useAgentSteeringOwners} from "../AgentSteeringProvider";
import {captureQueueOwnerIntent} from "../captureQueueOwnerIntent";

/** UI attaches to an app-owned transfer; durable recovery never sends a steer. */
export function useConversationSteeringHandoff(id:string,gateway:AgentContextGateway,registry:AgentSteeringHandoffRegistry|undefined,
  ready:boolean,projection:AgentConversationProjection|null,onSettled:()=>void|boolean|Promise<void|boolean>,scope:AgentQueueEditScope={kind:"context"},queueReadSequence=0,isAdmissionDisabled?:()=>boolean){
  const controller=useRef<AgentSteeringHandoffController|null>(null);
  const scopeKey=JSON.stringify(scope);
  const ordering=useAgentSteeringOwners()?.pendingOrder;
  const orderKey=scope.kind==="manuscript"?JSON.stringify([scope.manuscriptId,id]):id;
  const current=useRef({ready,projection,onSettled,scope,id,gateway,registry,scopeKey,isAdmissionDisabled});current.current={ready,projection,onSettled,scope,id,gateway,registry,scopeKey,isAdmissionDisabled};
  type SyncIdentity={id:string;gateway:AgentContextGateway;registry:AgentSteeringHandoffRegistry|undefined;scopeKey:string;owner:AgentSteeringHandoffController;sequence:number};
  const [synchronized,setSynchronized]=useState<SyncIdentity|null>(null);
  const syncing=useRef<SyncIdentity|null>(null);
  const available=!!gateway.steeringHandoff && (registry ? registry.ownsGateway(gateway.steeringHandoff) : scope.kind==="context" && !!gateway.steer);
  const [attached,setAttached]=useState<{id:string;gateway:AgentContextGateway;registry:AgentSteeringHandoffRegistry|undefined;scopeKey:string;owner:AgentSteeringHandoffController;state:AgentSteeringHandoffState}|null>(null);
  const match=attached?.id===id && attached.gateway===gateway && attached.registry===registry && attached.scopeKey===scopeKey;
  const state=match ? attached.state : null, activeOwner=match ? attached.owner : null;
  useEffect(()=>{
    setAttached(null);if(!available)return;
    const lease=registry?.acquire(id);
    const owned=registry?lease?.controller:gateway.steeringHandoff&&gateway.steer?new AgentSteeringHandoffController(id,gateway.steeringHandoff,gateway.steer,()=>crypto.randomUUID(),undefined,captureQueueOwnerIntent):undefined;
    if(!owned)return;
    const publish=()=>{const snapshot=owned.getSnapshot();
      const intents=[...snapshot.items.map(item=>item.intent),...(snapshot.intent?[snapshot.intent]:[])];
      ordering?.commit(orderKey,id,intents.map(intent=>`${intent.turnId}/${intent.steeringInputId}`));
      setAttached({id,gateway,registry,scopeKey,owner:owned,state:snapshot});};
    controller.current=owned;publish();
    const unsubscribe=owned.subscribe(publish);
    return()=>{unsubscribe();if(lease)lease.release();else owned.dispose();controller.current=null;};
  },[id,gateway,registry,available,scopeKey,ordering,orderKey]);
  async function refresh():Promise<void>{
    const owned=controller.current,view=current.current;
    if(!owned||!view.ready||!view.projection)return;
    const settled=await owned.refresh(turnId=>view.projection?.turns.get(turnId)?.outcome!=null);
    if(settled && controller.current===owned && !(owned.getSnapshot().settledSequence))void current.current.onSettled();
  }
  const settledSequence=state?.settledSequence??0;
  function sameIdentity(value:SyncIdentity|null,owner:AgentSteeringHandoffController|null):boolean{
    return !!value && value.id===id && value.gateway===gateway && value.registry===registry && value.scopeKey===scopeKey && value.owner===owner;
  }
  const syncedSequence=sameIdentity(synchronized,activeOwner)?synchronized!.sequence:0;
  const queueSyncPending=settledSequence>syncedSequence;
  useEffect(()=>{
    if(!ready||!state||!activeOwner||!queueSyncPending||sameIdentity(syncing.current,activeOwner))return;
    const attempt={id,gateway,registry,scopeKey,owner:activeOwner,sequence:settledSequence};syncing.current=attempt;
    void Promise.resolve().then(()=>current.current.onSettled()).then(success=>{
      if(syncing.current===attempt)syncing.current=null;
      const live=current.current;
      if(success!==false && controller.current===attempt.owner && live.id===attempt.id && live.gateway===attempt.gateway
        && live.registry===attempt.registry && live.scopeKey===attempt.scopeKey)setSynchronized(attempt);
    }).catch(()=>undefined).finally(()=>{if(syncing.current===attempt)syncing.current=null;});
  },[id,gateway,registry,scopeKey,activeOwner,ready,state,settledSequence,syncedSequence,queueReadSequence]);
  const observed=[...(state?.items.map(item=>item.intent)??[]),...(state?.intent?[state.intent]:[])];
  const observation=JSON.stringify(observed.map(intent=>{const turn=projection?.turns.get(intent.turnId),item=turn?.items.get(intent.steeringInputId);
    return [intent.operationId,turn?.outcome??null,item?.completed??false,item?.interrupted??false,item?.text===intent.text];}));
  useEffect(()=>{if(ready&&projection)void refresh();},[id,ready,!!projection,observation,gateway,registry,scopeKey]);
  async function submit(queue:AgentQueueState,turnId:string,inputId:string):Promise<boolean>{
    const owned=controller.current;if(!owned||!state||!owned.canSubmit(turnId)
      || (owned.getSnapshot().settledSequence??0)>(sameIdentity(synchronized,owned)?synchronized!.sequence:0))return false;
    const accepted=await owned.submit(queue,turnId,inputId,current.current.scope,current.current.isAdmissionDisabled);
    await refresh();return accepted;
  }
  async function retry():Promise<void>{
    const owned=controller.current,intent=state&&owned?.getSnapshot().intent;
    if(!owned||!intent)return;
    if(!current.current.ready || current.current.projection?.activeTurn!==intent.turnId){await refresh();return;}
    await owned.retry();await refresh();
  }
  return {available,state,blocked:available&&(!state||state.phase!=="ready"),queueSyncPending,
    canSubmit:(turnId:string)=>!queueSyncPending&&(controller.current?.canSubmit(turnId)??false),submit,retry,refresh};
}
