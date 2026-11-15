import {decodeAgentContextConfigurationEnvironmentRequest, decodeAgentContextConfigurationSaveIntent, decodeAgentContextConfigurationSeedIntent,
  type AgentContextConfigurationSaveIntent, type AgentContextConfigurationSeedIntent} from "@research-assistant/core/agent";
import type {ContextConfigurationOutbox} from "@research-assistant/data/agent";
import {appStorageScopedKey} from "../../../runtime/appStorage";
import {flushStorage, storagePersistence} from "../../../runtime/storagePersistence";
import {assertConversationOperationStorageCurrent, browserConversationOperationLock, type ConversationOperationLock} from "../conversationOperationLock";

interface FailedWrite {readonly before:string|null;readonly target:string|null}
const failurePort = Symbol.for("research-assistant.context-configuration-outbox-failures.v1");
const lockNamesPort = Symbol.for("research-assistant.context-configuration-outbox-lock-names.v1");
type OutboxStorage = Storage & {readonly [failurePort]?:Map<string,FailedWrite>; readonly [lockNamesPort]?:Map<string,string>};
const browserLock = browserConversationOperationLock("CONTEXT_CONFIGURATION_RECOVERY", {waitMs: 10000});
const prefix = "CONTEXT_CONFIGURATION_RECOVERY";

/** Owns only the immutable transport intent; HBS separately preserves H and its ancestors. */
export class ContextConfigurationStorageOutbox implements ContextConfigurationOutbox {
  private readonly key:string;
  private readonly lockName:string;
  private readonly failures:Map<string,FailedWrite>;
  private active=false;
  private observed:string|null=null;
  private readonly conversationId:string;
  constructor(private readonly storage:Storage, conversationId:string, private readonly lock:ConversationOperationLock=browserLock) {
    this.conversationId=decodeAgentContextConfigurationEnvironmentRequest({operationId:conversationId,conversationId}).conversationId;
    this.key=`research-assistant:context-configuration-save:v1:${this.conversationId}`;
    let lockNames=(storage as OutboxStorage)[lockNamesPort];
    if(!lockNames){lockNames=new Map();Object.defineProperty(storage,lockNamesPort,{value:lockNames,configurable:true});}
    const lockName=lockNames.get(this.key)??appStorageScopedKey(storage,`research-assistant:context-configuration-save-lock:v1:${this.conversationId}`);
    lockNames.set(this.key,lockName);this.lockName=lockName;
    let failuresForStorage=(storage as OutboxStorage)[failurePort];
    if(!failuresForStorage){failuresForStorage=new Map();Object.defineProperty(storage,failurePort,{value:failuresForStorage,configurable:true});}
    this.failures=failuresForStorage;
  }
  async run<T>(operation:()=>Promise<T>):Promise<T> {
    return this.exclusive(operation);
  }
  /** Hold the transport lock while repairing shared storage for a related owner. */
  async runStorageRepair<T>(operation:()=>Promise<T>):Promise<T> {
    if(this.active)throw new Error(`${prefix}_REENTRANT`);
    return this.lock.run(this.lockName,async()=>{
      if(this.failures.has(this.key))throw new Error(`${prefix}_LOCAL_UNCONFIRMED`);
      return operation();
    });
  }
  private async exclusive<T>(operation:()=>Promise<T>):Promise<T> {
    if(this.active)throw new Error(`${prefix}_REENTRANT`);
    return this.lock.run(this.lockName,async()=>{
      if(this.failures.has(this.key))throw new Error(`${prefix}_LOCAL_UNCONFIRMED`);
      await assertConversationOperationStorageCurrent(this.storage,prefix);
      this.observed=this.storage.getItem(this.key);this.active=true;
      try{return await operation();}finally{this.active=false;}
    });
  }
  async read():Promise<AgentContextConfigurationSaveIntent|null> {
    this.requireActive();const raw=this.current();
    if(raw===null)return null;
    const intent=decodeAgentContextConfigurationSaveIntent(JSON.parse(raw));
    if(intent.conversationId!==this.conversationId)throw new Error(`${prefix}_SCOPE_MISMATCH`);
    return intent;
  }
  /** Classify a persisted record without inferring an outcome from its type. */
  async kind():Promise<"empty"|"save"|"seed"> {
    this.requireActive();const raw=this.current();
    if(raw===null)return "empty";
    const value:unknown=JSON.parse(raw);
    if(!value || typeof value!=="object" || Array.isArray(value))throw new Error(`${prefix}_INVALID`);
    if(Object.hasOwn(value,"state")){await this.read();return "save";}
    await this.readSeed();return "seed";
  }
  async write(value:AgentContextConfigurationSaveIntent|null):Promise<void> {
    const existing=await this.read();
    const intent=value===null?null:decodeAgentContextConfigurationSaveIntent(value);
    if(intent && intent.conversationId!==this.conversationId)throw new Error(`${prefix}_SCOPE_MISMATCH`);
    if(existing && intent && JSON.stringify(existing)!==JSON.stringify(intent))throw new Error(`${prefix}_CONFLICT`);
    if(!existing && !intent)throw new Error(`${prefix}_MISSING`);
    await this.replace(intent===null?null:JSON.stringify(intent));
  }
  /** Seed and save share the same record and lock; neither can replace the
   * other's unresolved request. Existing save records retain their format. */
  async readSeed():Promise<AgentContextConfigurationSeedIntent|null> {
    this.requireActive();const raw=this.current();
    if(raw===null)return null;
    const intent=decodeAgentContextConfigurationSeedIntent(JSON.parse(raw));
    if(intent.conversationId!==this.conversationId)throw new Error(`${prefix}_SCOPE_MISMATCH`);
    return intent;
  }
  async writeSeed(value:AgentContextConfigurationSeedIntent|null):Promise<void> {
    const existing=await this.readSeed();
    const intent=value===null?null:decodeAgentContextConfigurationSeedIntent(value);
    if(intent && intent.conversationId!==this.conversationId)throw new Error(`${prefix}_SCOPE_MISMATCH`);
    if(existing && intent && JSON.stringify(existing)!==JSON.stringify(intent))throw new Error(`${prefix}_CONFLICT`);
    if(!existing && !intent)throw new Error(`${prefix}_MISSING`);
    await this.replace(intent===null?null:JSON.stringify(intent));
  }
  private async replace(target:string|null):Promise<void> {
    this.requireActive();const before=this.current();
    if(before===target)return;
    try {
      this.install(target);this.observed=target;
      await flushStorage(this.storage);
      if(this.storage.getItem(this.key)!==target)throw new Error(`${prefix}_CONFLICT`);
    } catch(cause) {
      this.failures.set(this.key,{before,target});
      throw cause;
    }
  }
  /** Explicitly settle a failed local write before using this outbox again. */
  async repair():Promise<void> {
    if(this.active)throw new Error(`${prefix}_REENTRANT`);
    await this.lock.run(this.lockName,async()=>{
      const failed=this.failures.get(this.key);if(!failed)return;
      await storagePersistence(this.storage)?.retry();
      await assertConversationOperationStorageCurrent(this.storage,prefix);
      const actual=this.storage.getItem(this.key);
      if(actual!==failed.before && actual!==failed.target)throw new Error(`${prefix}_CONFLICT`);
      if(actual!==failed.target){this.install(failed.target);await flushStorage(this.storage);}
      if(this.storage.getItem(this.key)!==failed.target)throw new Error(`${prefix}_CONFLICT`);
      this.failures.delete(this.key);
    });
  }
  private requireActive():void {
    if(!this.active)throw new Error(`${prefix}_LOCK_REQUIRED`);
    if(this.failures.has(this.key))throw new Error(`${prefix}_LOCAL_UNCONFIRMED`);
  }
  private current():string|null {
    const value=this.storage.getItem(this.key);
    if(value!==this.observed)throw new Error(`${prefix}_CONFLICT`);
    return value;
  }
  private install(value:string|null):void {
    if(value===null)this.storage.removeItem(this.key);else this.storage.setItem(this.key,value);
  }
}
