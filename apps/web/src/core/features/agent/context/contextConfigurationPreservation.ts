import {decodeAgentContextConfigurationEnvironmentRequest, type AgentContextState} from "@research-assistant/core/agent";
import type {DocumentCheckpoint, DocumentPreservation, HbsDocumentAdapter} from "@research-assistant/core/hbs";
import {decodeDocumentCheckpoint} from "@research-assistant/data/hbs";
import {appStorageScopedKey} from "../../../runtime/appStorage";
import {flushStorage, storagePersistence} from "../../../runtime/storagePersistence";
import {createRecoveryDraft, loadRecoveryDraft, recoveryDraftKey, saveRecoveryDraft,
  type RecoveryDraftEnvelope, type RecoveryDraftIdentity} from "../../../services/recoveryDraftStore";
import type {ContextConfigurationStorageOutbox} from "./contextConfigurationOutbox";
import {assertConversationOperationStorageCurrent, browserConversationOperationLock, type ConversationOperationLock} from "../conversationOperationLock";

interface FailedCheckpoint {readonly before:string|null;readonly target:string}
const failurePort=Symbol.for("research-assistant.context-configuration-preservation-failures.v1");
const lockPort=Symbol.for("research-assistant.context-configuration-preservation-lock-names.v1");
type PreservationStorage=Storage & {readonly [failurePort]?:Map<string,FailedCheckpoint>; readonly [lockPort]?:Map<string,string>};
const browserLock=browserConversationOperationLock("CONTEXT_CONFIGURATION_PRESERVATION");

/** HBS S′ remains separate from the immutable transport outbox. */
export class ContextConfigurationPreservation implements DocumentPreservation<AgentContextState> {
  private readonly identity:RecoveryDraftIdentity;
  private readonly key:string;
  private readonly failures:Map<string,FailedCheckpoint>;
  private readonly lockName:string;
  private loaded=false;
  private observed:string|null=null;
  constructor(private readonly storage:Storage, conversationId:string, private readonly outbox:ContextConfigurationStorageOutbox,
    private readonly epoch:()=>string|null, private readonly adapter:HbsDocumentAdapter<AgentContextState>,
    private readonly lock:ConversationOperationLock=browserLock) {
    const id=decodeAgentContextConfigurationEnvironmentRequest({operationId:conversationId,conversationId}).conversationId;
    this.identity={scope:"agent-context-hbs",resourceId:id,slot:"configuration"};
    this.key=recoveryDraftKey(this.identity);
    let locks=(storage as PreservationStorage)[lockPort];
    if(!locks){locks=new Map();Object.defineProperty(storage,lockPort,{value:locks,configurable:true});}
    this.lockName=locks.get(this.key)??appStorageScopedKey(storage,`research-assistant:context-configuration-preservation-lock:v1:${id}`);
    locks.set(this.key,this.lockName);
    let failures=(storage as PreservationStorage)[failurePort];
    if(!failures){failures=new Map();Object.defineProperty(storage,failurePort,{value:failures,configurable:true});}
    this.failures=failures;
  }
  async load():Promise<DocumentCheckpoint<AgentContextState>|undefined> {
    return this.exclusive(async()=>{
      this.requireClear();const epoch=this.requireEpoch();
      const raw=this.storage.getItem(this.key),draft=loadRecoveryDraft(this.identity,this.storage);
      if(this.storage.getItem(this.key)!==raw)throw new Error("CONTEXT_CONFIGURATION_PRESERVATION_CHANGED");
      if(draft && draft.baseHash!==epoch)throw new Error("CONTEXT_CONFIGURATION_PRESERVATION_EPOCH_MISMATCH");
      const value=draft?decodeDocumentCheckpoint(draft.payload,this.adapter):undefined;
      this.observed=raw;this.loaded=true;return value;
    });
  }
  async save(value:DocumentCheckpoint<AgentContextState>):Promise<void> {
    await this.exclusive(async()=>{
      this.requireClear();const epoch=this.requireEpoch();
      if(!this.loaded)throw new Error("CONTEXT_CONFIGURATION_PRESERVATION_NOT_LOADED");
      const before=this.storage.getItem(this.key);
      if(before!==this.observed)throw new Error("CONTEXT_CONFIGURATION_PRESERVATION_CHANGED");
      const checkpoint=decodeDocumentCheckpoint(value,this.adapter);
      const draft=createRecoveryDraft(this.identity,checkpoint,{revision:checkpoint.observed.version,hash:epoch});
      const target=JSON.stringify(draft);
      try{
        saveRecoveryDraft(draft,this.storage);await flushStorage(this.storage);
        if(this.storage.getItem(this.key)!==target)throw new Error("CONTEXT_CONFIGURATION_PRESERVATION_CHANGED");
        this.observed=target;
      }catch(cause){
        // A validation rejection before the storage changes must not trap the
        // user on an impossible checkpoint (for example an oversized draft).
        // A changed view, a backend failure or an unreadable view stays fenced.
        let uncertain=true;
        try{uncertain=this.storage.getItem(this.key)!==before || !!storagePersistence(this.storage)?.error;}catch{/* Remain fenced. */}
        if(uncertain)this.failures.set(this.key,{before,target});
        throw cause;
      }
    });
  }
  /** Settles only the original failed S′ write; it never commits S/B. */
  async repair():Promise<void> {
    await this.exclusive(async()=>{
      const failed=this.failures.get(this.key);if(!failed)return;
      const draft=JSON.parse(failed.target) as RecoveryDraftEnvelope<unknown>;
      if(draft.baseHash!==this.requireEpoch())throw new Error("CONTEXT_CONFIGURATION_PRESERVATION_EPOCH_MISMATCH");
      decodeDocumentCheckpoint(draft.payload,this.adapter);
      const actual=this.storage.getItem(this.key);
      if(actual!==failed.before && actual!==failed.target)throw new Error("CONTEXT_CONFIGURATION_PRESERVATION_CHANGED");
      if(actual!==failed.target){
        saveRecoveryDraft(draft,this.storage);await flushStorage(this.storage);
      }
      if(this.storage.getItem(this.key)!==failed.target)throw new Error("CONTEXT_CONFIGURATION_PRESERVATION_CHANGED");
      this.observed=failed.target;this.loaded=true;this.failures.delete(this.key);
    },true);
  }
  private async exclusive<T>(operation:()=>Promise<T>,retryStorage=false):Promise<T> {
    return this.lock.run(this.lockName,async()=>{
      if(retryStorage){
        return this.outbox.runStorageRepair(async()=>{
          await storagePersistence(this.storage)?.retry();
          await assertConversationOperationStorageCurrent(this.storage,"CONTEXT_CONFIGURATION_PRESERVATION");
          return operation();
        });
      }
      await assertConversationOperationStorageCurrent(this.storage,"CONTEXT_CONFIGURATION_PRESERVATION");
      return operation();
    });
  }
  private requireEpoch():string {const value=this.epoch();if(!value)throw new Error("CONTEXT_CONFIGURATION_PRESERVATION_EPOCH_REQUIRED");return value;}
  private requireClear():void {if(this.failures.has(this.key))throw new Error("CONTEXT_CONFIGURATION_PRESERVATION_LOCAL_UNCONFIRMED");}
}
