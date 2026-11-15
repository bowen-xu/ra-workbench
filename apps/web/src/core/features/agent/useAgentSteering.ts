import type {AgentConversationProjection,AgentUserInput} from "@research-assistant/core/agent";
import {useEffect,useRef,useState} from "react";
import {AgentSteeringController,AgentSteeringRegistry,type AgentSteeringState} from "@research-assistant/data/agent";
import {useAgentSteeringOwners} from "./AgentSteeringProvider";
import {captureQueueOwnerIntent} from "./captureQueueOwnerIntent";

export interface AgentSteeringOwner<S extends {conversationId:string;turnId:string}> {
  readonly registry:AgentSteeringRegistry<S>;
  readonly hostKey:string;
  readonly conversationId:string;
  readonly captureAcknowledgment:(text:string)=>()=>void;
  readonly queueRejected?: (request:{readonly inputId:string;readonly text:string;readonly userInput?:readonly AgentUserInput[]},acknowledge?:()=>void)=>Promise<boolean>;
}
export interface AgentSteeringSubmitCallbacks {
  readonly userInput?:readonly AgentUserInput[];
  readonly accepted?:()=>void;
  readonly confirmed?:()=>void|boolean|Promise<void|boolean>;
  readonly queueRejected?:AgentSteeringOwner<{conversationId:string;turnId:string}>["queueRejected"];
  /** Definitive first-request rejection only; unknown outcomes remain attached
   * for retry/history reconciliation. */
  readonly rejected?:()=>void;
}
/** UI attaches to an application owner; standalone hooks keep their previous local lifetime. */
export function useAgentSteering<S extends {conversationId:string;turnId:string}>(scope:S|null,key:string,send:(request:S&{inputId:string;text:string})=>Promise<unknown>,accepted:(text:string)=>void,projection:AgentConversationProjection|null=null,owner?:AgentSteeringOwner<S>,historyGate?:{readonly isReady:()=>boolean;readonly state:unknown},isAdmissionDisabled?:()=>boolean){
  const ordering=useAgentSteeringOwners()?.pendingOrder;
  const orderIdentity=useRef({key:"",conversationId:""});
  if(owner || scope)orderIdentity.current={key:owner?.hostKey??scope?.conversationId??"",conversationId:owner?.conversationId??scope?.conversationId??""};
  const current=useRef({key,accepted});current.current={key,accepted};
  const controller=useRef<AgentSteeringController<S>|null>(null);
  const pendingKey=useRef("");
  const delivered=useRef<string|null>(null);
  const [state,setState]=useState<AgentSteeringState<S>>({phase:"idle",request:null,status:""});
  useEffect(()=>{
    const lease=owner?.registry.acquire(owner.hostKey,owner.conversationId);
    const owned=owner?lease?.controller:null;
    const attached=owned??(!owner?new AgentSteeringController<S>(send,()=>crypto.randomUUID(),captureQueueOwnerIntent):null);
    if(!attached){controller.current=null;return;}
    controller.current=attached;
    const publish=()=>{const snapshot=attached.getSnapshot();
      const {key:orderKey,conversationId}=orderIdentity.current;
      if(orderKey && conversationId)ordering?.commit(orderKey,conversationId,(snapshot.awaitingHistory??[]).map(item=>`${item.turnId}/${item.inputId}`));
      setState(snapshot);};
    publish();const unsubscribe=attached.subscribe(publish);
    return()=>{unsubscribe();if(lease)lease.release();else attached.dispose();controller.current=null;};
  },[send,owner?.registry,owner?.hostKey,owner?.conversationId,ordering]);
  function deliver(inputId:string|undefined,text:string,requestKey:string):void{
    if(owner)return;
    if(!inputId||delivered.current===inputId||current.current.key!==requestKey)return;
    delivered.current=inputId;current.current.accepted(text);
  }
  useEffect(()=>{
    if(!projection || (historyGate && !historyGate.isReady()))return;
    const owned=controller.current,inputId=owned?.getSnapshot().request?.inputId;
    const text=owned?.observeHistory(projection);
    if(text!==undefined&&text!==null)deliver(inputId,text,pendingKey.current);
  },[projection,state.phase,historyGate?.state]);
  async function retry(): Promise<boolean> {
    const owned=controller.current;if(!owned)return false;
    const text=owned.getSnapshot().request?.text;
    const accepted = await owned.retry();
    if(accepted&&text!==undefined)deliver(owned.getSnapshot().request?.inputId,text,pendingKey.current);
    return accepted;
  }
  function retryConfirmation(inputId:string):void { controller.current?.retryConfirmation(inputId); }
  async function submit(text:string,callbacks?:AgentSteeringSubmitCallbacks): Promise<boolean>{
    const owned=controller.current;if(!owned||scope===null){callbacks?.rejected?.();return false;}
    const previous=owned.getSnapshot();
    if(previous.phase==="uncertain"){
      if(previous.request?.text===text&&JSON.stringify(previous.request?.userInput??[])===JSON.stringify(callbacks?.userInput??[])&&pendingKey.current===key)return retry();
      callbacks?.rejected?.();
      return false;
    }
    if(previous.phase==="sending"){callbacks?.rejected?.();return false;}
    pendingKey.current=key;
    const ownerAcknowledgment=owner?.captureAcknowledgment(text);
    const acknowledgment=ownerAcknowledgment||callbacks?.accepted
      ? ()=>{ownerAcknowledgment?.();callbacks?.accepted?.();}
      : undefined;
    const fallback=callbacks?.queueRejected??owner?.queueRejected;
    const accepted = await owned.submit(scope,text,acknowledgment,callbacks?.rejected,callbacks?.userInput,callbacks?.confirmed,fallback?request=>fallback(request,ownerAcknowledgment):undefined,isAdmissionDisabled);
    if(accepted)deliver(owned.getSnapshot().request?.inputId,text,key);
    return accepted;
  }
  const unresolved=state.phase==="sending"||state.phase==="uncertain";
  return {submit,retry,retryConfirmation,state,unresolved,pending:state.phase==="sending",uncertain:state.phase==="uncertain",status:unresolved||pendingKey.current===key?state.status:""};
}
