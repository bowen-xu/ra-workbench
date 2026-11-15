import {decodeAgentContextState, type AgentContextState} from "@research-assistant/core/agent";
import {createStructuredDocumentAdapter, HbsSession, type HbsDocumentAdapter} from "@research-assistant/core/hbs";
import {createContextConfigurationDocumentPort, type AgentContextConfigurationSaveGateway} from "@research-assistant/data/agent";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import type {ConversationOperationLock} from "../conversationOperationLock";
import {ContextConfigurationStorageOutbox} from "./contextConfigurationOutbox";
import {ContextConfigurationPreservation} from "./contextConfigurationPreservation";

/** Composition only: the supplied host owns the session lifetime and recovery UI. */
export async function openContextConfigurationHbs(options:{readonly storage:Storage;readonly conversationId:string;
  readonly gateway:AgentContextConfigurationSaveGateway;readonly newId:()=>string;readonly lock?:ConversationOperationLock;readonly canWrite?:()=>boolean}) {
  const structured=createStructuredDocumentAdapter(directTextDiff);
  const adapter={...structured,decode:decodeAgentContextState} as HbsDocumentAdapter<AgentContextState>;
  const outbox=new ContextConfigurationStorageOutbox(options.storage,options.conversationId,options.lock);
  let lastFailure:unknown;
  const gateway:AgentContextConfigurationSaveGateway={...options.gateway,
    save:async intent=>{try{const proof=await options.gateway.save(intent);lastFailure=undefined;return proof;}catch(reason){lastFailure=reason;throw reason;}},
    status:async intent=>{try{return await options.gateway.status(intent);}catch(reason){lastFailure=reason;throw reason;}},
  };
  const port=createContextConfigurationDocumentPort(options.conversationId,gateway,outbox,options.newId,options.canWrite);
  const preservation=new ContextConfigurationPreservation(options.storage,options.conversationId,outbox,port.dataEpoch,adapter,options.lock);
  const session=await HbsSession.open({adapter,port,preservation,requestId:options.newId});
  return {session,port,outbox,preservation,lastFailure:()=>lastFailure};
}
