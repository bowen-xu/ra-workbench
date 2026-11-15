// @vitest-environment happy-dom
import {beforeEach, expect, it, vi} from "vitest";
import type {ConversationOperationLock} from "../conversationOperationLock";
import {ContextConfigurationStorageOutbox} from "./contextConfigurationOutbox";
import {createIndexedPreviewStorage} from "../../../runtime/previewIndexedStorage";
import {configurePreviewDatabase} from "../../../runtime/previewIndexedDatabase";
import {createMemoryPreviewDatabase} from "../../../../app/runtime/testing/memoryPreviewDatabase";
import {flushStorage} from "../../../runtime/storagePersistence";
import {createContextConfigurationDocumentPort, type AgentContextConfigurationSaveGateway} from "@research-assistant/data/agent";
import type {AgentContextConfigurationSaveIntent} from "@research-assistant/core/agent";
import {createMockAgentRecoveryStorage} from "../../../../app/runtime/mock/MockAgentRecoveryStorage";

const conversationId="11111111-1111-4111-8111-111111111111", operationId="22222222-2222-4222-8222-222222222222", expectedEpoch="33333333-3333-4333-8333-333333333333";
const intent={conversationId,operationId,expectedEpoch,expectedRevision:1,state:{schemaVersion:1 as const,persistent:[],temporary:[]}};
let database:ReturnType<typeof createMemoryPreviewDatabase>;
let lock:ConversationOperationLock;
beforeEach(()=>{
  database=createMemoryPreviewDatabase();configurePreviewDatabase(database.open);
  const held=new Set<string>();
  lock={async run<T>(name:string,operation:()=>Promise<T>):Promise<T>{
    if(held.has(name))throw Error("LOCK_BUSY");held.add(name);
    try{return await operation();}finally{held.delete(name);}
  }};
});

it("persists the exact intent and discovers it after opening the same preview store",async()=>{
  const storage=await createIndexedPreviewStorage("context-outbox:"),outbox=new ContextConfigurationStorageOutbox(storage,conversationId,lock);
  await outbox.run(async()=>{expect(await outbox.read()).toBeNull();await outbox.write(intent);});
  const reopened=await createIndexedPreviewStorage("context-outbox:"),reader=new ContextConfigurationStorageOutbox(reopened,conversationId,lock);
  expect(await reader.run(()=>reader.read())).toEqual(intent);
  await expect(reader.run(()=>reader.write({...intent,operationId:expectedEpoch}))).rejects.toThrow("CONTEXT_CONFIGURATION_RECOVERY_CONFLICT");
});

it("uses the same locked outbox with the desktop localStorage port",async()=>{
  localStorage.clear();
  const first=new ContextConfigurationStorageOutbox(localStorage,conversationId,lock),second=new ContextConfigurationStorageOutbox(localStorage,conversationId,lock);
  await first.run(()=>first.write(intent));expect(await second.run(()=>second.read())).toEqual(intent);
  await second.run(()=>second.write(null));expect(await first.run(()=>first.read())).toBeNull();
  localStorage.clear();
});

it("repairs an original localStorage write that failed before changing the value",async()=>{
  let failing=true;
  const storage=createMockAgentRecoveryStorage("context-write",()=>{if(failing)throw Error("quota");});
  const outbox=new ContextConfigurationStorageOutbox(storage,conversationId,lock);
  await expect(outbox.run(()=>outbox.write(intent))).rejects.toThrow("quota");
  await expect(outbox.run(()=>outbox.read())).rejects.toThrow("CONTEXT_CONFIGURATION_RECOVERY_LOCAL_UNCONFIRMED");
  failing=false;await outbox.repair();expect(await outbox.run(()=>outbox.read())).toEqual(intent);
});

it("fails closed after a pending write loses its local receipt and explicitly repairs it",async()=>{
  const storage=await createIndexedPreviewStorage("context-failed-write:"),first=new ContextConfigurationStorageOutbox(storage,conversationId,lock),second=new ContextConfigurationStorageOutbox(storage,conversationId,lock);
  database.failNext(new DOMException("full","QuotaExceededError"));
  await expect(first.run(()=>first.write(intent))).rejects.toThrow();
  await expect(second.run(()=>second.read())).rejects.toThrow("CONTEXT_CONFIGURATION_RECOVERY_LOCAL_UNCONFIRMED");
  const before=await createIndexedPreviewStorage("context-failed-write:"),reader=new ContextConfigurationStorageOutbox(before,conversationId,lock);
  expect(await reader.run(()=>reader.read())).toBeNull();
  await first.repair();
  const after=await createIndexedPreviewStorage("context-failed-write:"),recovered=new ContextConfigurationStorageOutbox(after,conversationId,lock);
  expect(await recovered.run(()=>recovered.read())).toEqual(intent);
});

it("retains an unresolved local write across module reload on the same storage",async()=>{
  const names:string[]=[];
  const observedLock:ConversationOperationLock={run:(name,operation)=>{names.push(name);return lock.run(name,operation);}};
  const storage=await createIndexedPreviewStorage("context-module-reload:"),original=new ContextConfigurationStorageOutbox(storage,conversationId,observedLock);
  database.failNext(new DOMException("full","QuotaExceededError"));
  await expect(original.run(()=>original.write(intent))).rejects.toThrow();
  vi.resetModules();
  const {ContextConfigurationStorageOutbox:Reloaded}=await import("./contextConfigurationOutbox");
  const reloaded=new Reloaded(storage,conversationId,observedLock);
  await expect(reloaded.run(()=>reloaded.read())).rejects.toThrow("CONTEXT_CONFIGURATION_RECOVERY_LOCAL_UNCONFIRMED");
  await reloaded.repair();expect(await original.run(()=>original.read())).toEqual(intent);
  expect(new Set(names).size).toBe(1);
});

it("continues to fence reads inside the same locked operation after a write failure",async()=>{
  const storage=await createIndexedPreviewStorage("context-inner-failure:"),outbox=new ContextConfigurationStorageOutbox(storage,conversationId,lock);
  database.failNext(new DOMException("full","QuotaExceededError"));
  await outbox.run(async()=>{
    await expect(outbox.write(intent)).rejects.toThrow();
    await expect(outbox.read()).rejects.toThrow("CONTEXT_CONFIGURATION_RECOVERY_LOCAL_UNCONFIRMED");
  });
  await outbox.repair();expect(await outbox.run(()=>outbox.read())).toEqual(intent);
});

it("durably records the HBS request before calling the native save port",async()=>{
  const storage=await createIndexedPreviewStorage("context-hbs-outbox:"),outbox=new ContextConfigurationStorageOutbox(storage,conversationId,lock);
  const gateway:AgentContextConfigurationSaveGateway={
    environment:async request=>({schemaVersion:1,...request,epoch:expectedEpoch,sqlFence:"0",configuration:null}),
    status:async()=>({type:"noSqlReceipt",epoch:expectedEpoch,sqlFence:"0",closed:false}),
    save:vi.fn(async (original:AgentContextConfigurationSaveIntent)=>{
      const reopened=await createIndexedPreviewStorage("context-hbs-outbox:");
      // Inspect the isolated durable store while the original lock is held.
      const key=`research-assistant:context-configuration-save:v1:${conversationId}`;
      expect(JSON.parse(reopened.getItem(key)!)).toEqual(original);
      return {schemaVersion:1 as const,operationId:original.operationId,conversationId,epoch:expectedEpoch,family:"agent.context.configuration.save" as const,sqlFence:"0",replayed:false,result:null};
    }),
  };
  const port=createContextConfigurationDocumentPort(conversationId,gateway,outbox,()=>operationId);
  await port.read();await port.commit({id:operationId,expectedVersion:0,saved:intent.state,baseline:intent.state});
  expect(gateway.save).toHaveBeenCalledOnce();
  const reopened=await createIndexedPreviewStorage("context-hbs-outbox:"),reader=new ContextConfigurationStorageOutbox(reopened,conversationId,lock);
  expect(await reader.run(()=>reader.read())).toBeNull();
});

it("does not treat a failed clear as durable and repairs only the original clear",async()=>{
  const storage=await createIndexedPreviewStorage("context-failed-clear:"),outbox=new ContextConfigurationStorageOutbox(storage,conversationId,lock);
  await outbox.run(()=>outbox.write(intent));database.failNext(new DOMException("full","QuotaExceededError"));
  await expect(outbox.run(()=>outbox.write(null))).rejects.toThrow();
  await expect(outbox.run(()=>outbox.read())).rejects.toThrow("CONTEXT_CONFIGURATION_RECOVERY_LOCAL_UNCONFIRMED");
  const old=await createIndexedPreviewStorage("context-failed-clear:"),oldReader=new ContextConfigurationStorageOutbox(old,conversationId,lock);
  expect(await oldReader.run(()=>oldReader.read())).toEqual(intent);
  await outbox.repair();
  const after=await createIndexedPreviewStorage("context-failed-clear:"),reader=new ContextConfigurationStorageOutbox(after,conversationId,lock);
  expect(await reader.run(()=>reader.read())).toBeNull();
});

it("rejects a stale hydrated store and preserves the other window's record",async()=>{
  const first=await createIndexedPreviewStorage("context-stale:"),stale=await createIndexedPreviewStorage("context-stale:");
  const writer=new ContextConfigurationStorageOutbox(first,conversationId,lock),other=new ContextConfigurationStorageOutbox(stale,conversationId,lock);
  await writer.run(()=>writer.write(intent));
  await expect(other.run(()=>other.read())).rejects.toThrow("CONTEXT_CONFIGURATION_RECOVERY_STORAGE_STALE");
  const reopened=await createIndexedPreviewStorage("context-stale:"),reader=new ContextConfigurationStorageOutbox(reopened,conversationId,lock);
  expect(await reader.run(()=>reader.read())).toEqual(intent);
});

it("keeps the lock across the awaited operation and rejects access outside it",async()=>{
  const storage=await createIndexedPreviewStorage("context-lock:"),first=new ContextConfigurationStorageOutbox(storage,conversationId,lock),second=new ContextConfigurationStorageOutbox(storage,conversationId,lock);
  await expect(first.read()).rejects.toThrow("CONTEXT_CONFIGURATION_RECOVERY_LOCK_REQUIRED");
  let release!:()=>void;const action=vi.fn(async()=>{await new Promise<void>(resolve=>{release=resolve;});await first.write(intent);});
  const pending=first.run(action);while(!release)await Promise.resolve();
  await expect(second.run(()=>second.read())).rejects.toThrow("LOCK_BUSY");
  release();await pending;
  expect(await second.run(()=>second.read())).toEqual(intent);
});

it("does not replace a corrupt existing recovery record",async()=>{
  const storage=await createIndexedPreviewStorage("context-corrupt:"),key=`research-assistant:context-configuration-save:v1:${conversationId}`;
  storage.setItem(key,"{broken");await flushStorage(storage);
  const outbox=new ContextConfigurationStorageOutbox(storage,conversationId,lock);
  await expect(outbox.run(()=>outbox.write(intent))).rejects.toThrow();
  expect(storage.getItem(key)).toBe("{broken");
});
