import {expect,it,vi} from "vitest";
import {agentDraftInput,insertAgentReference, type AgentReferenceInput} from "@research-assistant/core/agent";
import {ConversationDraftStore,conversationDraftKey} from "./conversationDraftStore";
import {localConversationDraftPersistence} from "./conversationDraftPersistence";
const key=conversationDraftKey({hostKind:"paper",hostId:"paper-a",conversationId:"chat-a"});
const skill:AgentReferenceInput={type:"skill",name:"review",identity:{environment_id:"application",source_id:".agents/skills/review/SKILL.md",revision:"v1"}};
it("restores exact bindings after a provider restart and forgets deleted chats",()=>{
 let value:string|null=null;const storage={getItem:()=>value,setItem:(_key:string,text:string)=>{value=text;}};
 const first=new ConversationDraftStore(localConversationDraftPersistence(storage));first.updateStructured(key,insertAgentReference({text:"😀 review",bindings:[]},skill));
 const second=new ConversationDraftStore(localConversationDraftPersistence(storage));expect(agentDraftInput(second.readStructured(key))).toEqual(agentDraftInput(first.readStructured(key)));
 second.update(key,second.read(key).replace("@review","@changed"));second.update(key,first.read(key));expect(second.readStructured(key).bindings).toEqual([]);
 second.forgetConversation("chat-a");expect(new ConversationDraftStore(localConversationDraftPersistence(storage)).read(key)).toBe("");
});
it("preserves unreadable stored bytes and keeps failed writes in the current draft",()=>{
 const save=vi.fn();const broken=new ConversationDraftStore({load:()=>{throw new Error("corrupt");},save});broken.update(key,"new draft");expect(save).not.toHaveBeenCalled();expect(broken.error()).toContain("仅保存在内存");
 const failed=new ConversationDraftStore({load:()=>new Map(),save:()=>{throw new Error("full");}});failed.updateStructured(key,insertAgentReference({text:"",bindings:[]},skill));expect(failed.readStructured(key).bindings).toHaveLength(1);expect(failed.error()).toContain("暂未保存");
});
it("rejects malformed persisted identities and budgets",()=>{
 const persistence=localConversationDraftPersistence({getItem:()=>JSON.stringify({version:1,drafts:[[key,"x",[{type:"text",text:"different",textElements:[]}]]]}),setItem:()=>undefined});expect(()=>persistence.load()).toThrow();
});
it("restores a pending host draft and migrates the exact revision when its conversation is created",()=>{
 let value:string|null=null;const storage={getItem:()=>value,setItem:(_key:string,text:string)=>{value=text;}};
 const scope={hostKind:"note" as const,hostId:"note-a"};const pending=conversationDraftKey({...scope,conversationId:null});const bound=conversationDraftKey({...scope,conversationId:"created"});
 const first=new ConversationDraftStore(localConversationDraftPersistence(storage));first.updateStructured(pending,insertAgentReference({text:"",bindings:[]},skill));
 const restored=new ConversationDraftStore(localConversationDraftPersistence(storage));const input=agentDraftInput(restored.readStructured(pending));const receipt=restored.captureAcknowledgment(pending,restored.read(pending).trim());
 restored.adoptPending(scope,"created");expect(restored.read(pending)).toBe("");expect(agentDraftInput(restored.readStructured(bound))).toEqual(input);
 expect(agentDraftInput(new ConversationDraftStore(localConversationDraftPersistence(storage)).readStructured(bound))).toEqual(input);
 restored.update(pending,"a new pending message");receipt();expect(restored.read(bound)).toBe("");expect(restored.read(pending)).toBe("a new pending message");
});
it("does not clear a draft edited after adoption or overwrite an existing bound draft",()=>{
 const store=new ConversationDraftStore();const scope={hostKind:"manuscript" as const,hostId:"work"};const pending=conversationDraftKey({...scope,conversationId:null});const bound=conversationDraftKey({...scope,conversationId:"created"});
 store.update(pending,"before");const receipt=store.captureAcknowledgment(pending,"before");store.adoptPending(scope,"created");store.update(bound,"after");receipt();expect(store.read(bound)).toBe("after");
 store.update(pending,"second");store.adoptPending(scope,"created");expect(store.read(bound)).toBe("after");expect(store.read(pending)).toBe("second");
});
it("preserves corrupt v2 and duplicate revisions without publishing partial content or overwriting original bytes", async () => {
 const id=(n:number)=>`00000000-0000-4000-8000-${String(n).padStart(12,"0")}`;
 const other=conversationDraftKey({hostKind:"paper",hostId:"paper-a",conversationId:"chat-b"});
 const row=(scopeKey:string, revision:string)=>({key:scopeKey,text:"original",userInput:[],queuePlacement:null,revision,originKey:scopeKey,sourceQueueRevision:null});
 for(const drafts of [[row(key,id(1)),row(other,id(1))],[row(key,id(1)),{...row(other,id(2)),originKey:conversationDraftKey({hostKind:"note",hostId:"foreign",conversationId:null})}],
  [row(key,id(1)),{...row(other,id(2)),revision:"bad"}],[row(key,id(1)),row(key,id(2))]]){
  const original=JSON.stringify({version:2,drafts});let bytes=original;const save=vi.fn((_key:string,value:string)=>{bytes=value;});
  const store=new ConversationDraftStore(localConversationDraftPersistence({getItem:()=>bytes,setItem:save}));
  expect(store.read(key)).toBe("");expect(store.error()).toContain("无法恢复");store.update(key,"later local draft");
  await expect(store.flush()).rejects.toThrow("AGENT_DRAFT_RECOVERY_UNREADABLE");expect(save).not.toHaveBeenCalled();expect(bytes).toBe(original);
 }
});
it("rejects duplicate local queue-origin revisions independently of draft content revisions",()=>{
 const id=(n:number)=>`00000000-0000-4000-8000-${String(n).padStart(12,"0")}`;
 const other=conversationDraftKey({hostKind:"paper",hostId:"paper-a",conversationId:"chat-b"});
 const drafts=[key,other].map((scopeKey,index)=>({key:scopeKey,text:"original",userInput:[],revision:id(index+1),originKey:scopeKey,sourceQueueRevision:id(3),
  queuePlacement:{input:{inputId:id(index+10),text:"original"},index:0,preceding:[],following:[]}}));
 const persistence=localConversationDraftPersistence({getItem:()=>JSON.stringify({version:2,drafts}),setItem:()=>{}});
 expect(()=>persistence.load()).toThrow("草稿队列归属无效");
});
