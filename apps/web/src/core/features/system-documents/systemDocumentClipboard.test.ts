/** @vitest-environment happy-dom */
import {expect,it} from "vitest";
import {systemDocumentPath,systemDocumentReference,serializeAgentSourceReferences} from "@research-assistant/core/agent";
import {writeComposerClipboard,readComposerClipboard} from "../agent/agentComposerClipboard";
import {createMockSystemDocuments} from "../../../app/runtime/mock/mockSystemDocuments";
it("system id-path round-trips as a typed @file with no content or write grant",async()=>{
 const document=await createMockSystemDocuments(localStorage).gateway.read("portrait-reference");
 const draft={text:document.path,bindings:[{startUtf16:0,endUtf16:document.path.length,input:systemDocumentReference(document)}]};
 const formats=new Map<string,string>();const data={getData:(type:string)=>formats.get(type)??"",setData:(type:string,value:string)=>{formats.set(type,value);}};
 expect(writeComposerClipboard(data,draft)).toBe(true);expect(readComposerClipboard(data)).toEqual(draft);formats.delete("application/x-research-assistant-agent-draft+json");
 expect(readComposerClipboard(data)).toEqual(draft);
 expect(formats.get("text/html")).not.toContain(document.content);
 const wire=serializeAgentSourceReferences(draft,{kind:"note",resourceId:"page",conversationId:"chat"});
 expect(wire.text).toContain(systemDocumentPath(document.id));expect(wire.userInput).toContainEqual(systemDocumentReference(document));
 expect(readComposerClipboard({getData:type=>type==="text/plain"?document.path:""})).toBeNull();
});
