/** @vitest-environment happy-dom */
import {act,useState,type ReactNode} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {AgentTurnChangeSummary,AgentChangeContribution} from "./AgentTurnChangeSummary";
import {WritingAgentTranscript} from "./presentation/WritingAgentTranscript";
import {WritingWorkspaceChangeSummary} from "../../../../../../plugins/writing_projects/src/features/writing/agent/WritingWorkspaceReview";
import type {WritingAgentMessage} from "./presentation/WritingAgentMessage";
import {selectAppStorage} from "../../runtime/appStorage";

const mocks=vi.hoisted(()=>({notes:{review:vi.fn(),decide:vi.fn()},structured:{get:vi.fn(),getVersion:vi.fn(),review:vi.fn()},files:{latest:vi.fn(),review:vi.fn(),readFile:vi.fn()},notifyError:vi.fn(),observers:new Set<()=>void>()}));
vi.mock("../../services/notifications",()=>({notify:{error:mocks.notifyError}}));
vi.mock("./textPatchGateway",()=>({textPatchGateway:mocks.notes,subscribeTextPatches:(listener:()=>void)=>{mocks.observers.add(listener);return()=>mocks.observers.delete(listener);},notifyTextPatchesChanged:()=>mocks.observers.forEach(listener=>listener())}));
vi.mock("../../services/structuredResourceGateway",()=>({getStructuredResourceGateway:()=>mocks.structured}));
vi.mock("../../services/agentGateway",()=>({agentWorkspaceReviewGateway:mocks.files,agentProposalGateway:mocks.files}));
vi.mock("./presentation/WritingTextDiff",()=>({WritingTextDiff:()=>null}));
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
const cleanups:(()=>Promise<void>)[]=[];
beforeEach(()=>{vi.clearAllMocks();selectAppStorage(localStorage);localStorage.clear();});
afterEach(async()=>{for(const cleanup of cleanups.splice(0))await cleanup();});
async function mount(element:ReactNode){const host=document.createElement("div"),root=createRoot(host);document.body.append(host);cleanups.push(async()=>{await act(async()=>root.unmount());host.remove();});await act(async()=>root.render(element));return host;}
const item=(id:string)=>({id,identity:id,label:id,stats:{additions:1,deletions:0},onOpen:vi.fn()});
it("reports a thrown Undo failure through the shared notification and preserves confirmed owners",async()=>{
 const a=vi.fn().mockResolvedValue(true),b=vi.fn().mockRejectedValueOnce(new Error("revision conflict")).mockResolvedValue(true);
 const host=await mount(<AgentTurnChangeSummary><AgentChangeContribution ariaLabel="a" items={[item("a")]} undo={{disabled:false,run:a}}/><AgentChangeContribution ariaLabel="b" items={[item("b")]} undo={{disabled:false,run:b}}/></AgentTurnChangeSummary>);
 await act(async()=>host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")!.click());
 expect(mocks.notifyError).toHaveBeenCalledOnce();expect(mocks.notifyError).toHaveBeenCalledWith("撤销失败：revision conflict");
 expect(host.querySelector('[role="alert"]')).toBeNull();expect(host.textContent).not.toContain("revision conflict");
 expect(host.querySelector("header strong")?.textContent).toBe("Edited 2 files");
 await act(async()=>host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")!.click());
 expect(a).toHaveBeenCalledTimes(1);expect(b).toHaveBeenCalledTimes(2);expect(mocks.notifyError).toHaveBeenCalledOnce();
});
it("the parent turn owns one card even when nested adapters each declare a fallback scope",async()=>{
 const undoA=vi.fn().mockResolvedValue(true),undoB=vi.fn().mockResolvedValue(true);
 const host=await mount(<AgentTurnChangeSummary><AgentChangeContribution ariaLabel="file" items={[item("file")]} undo={{disabled:false,run:undoA}}/>
  <AgentTurnChangeSummary><AgentChangeContribution ariaLabel="note" items={[item("note")]} undo={{disabled:false,run:undoB}}/></AgentTurnChangeSummary></AgentTurnChangeSummary>);
 expect(host.querySelectorAll(".writing-agent-inline-diff-card")).toHaveLength(1);
 expect(host.querySelector("header strong")?.textContent).toBe("Edited 2 files");
 expect(host.querySelector(".writing-agent-inline-diff-counts b")?.textContent).toBe("+2");
 expect(host.querySelectorAll(".writing-agent-inline-diff-undo")).toHaveLength(1);
 await act(async()=>host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")!.click());
 expect(undoA).toHaveBeenCalledTimes(1);expect(undoB).toHaveBeenCalledTimes(1);
});
it("does not replay confirmed contributors when the remaining adapter needs recovery",async()=>{
 const a=vi.fn().mockResolvedValue(true),b=vi.fn().mockResolvedValueOnce(false).mockResolvedValue(true);
 const host=await mount(<AgentTurnChangeSummary><AgentChangeContribution ariaLabel="a" items={[item("a")]} undo={{disabled:false,run:a}}/><AgentChangeContribution ariaLabel="b" items={[item("b")]} undo={{disabled:false,run:b}}/></AgentTurnChangeSummary>);
 await act(async()=>host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")!.click());
 expect(host.textContent).toContain("部分修改已撤销");
 await act(async()=>host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")!.click());
 expect(a).toHaveBeenCalledTimes(1);expect(b).toHaveBeenCalledTimes(2);
 expect(host.textContent).not.toContain("部分修改已撤销");
 expect(host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")!.disabled).toBe(true);
});
it("dispatches one business owner's Undo once for all of its registered file contributions",async()=>{
 const first=vi.fn().mockResolvedValue(true),second=vi.fn().mockResolvedValue(true);
 const host=await mount(<AgentTurnChangeSummary><AgentChangeContribution ownerIdentity="original-proposal" ariaLabel="first" items={[item("first")]} undo={{disabled:false,run:first}}/>
  <AgentChangeContribution ownerIdentity="original-proposal" ariaLabel="second" items={[item("second")]} undo={{disabled:false,run:second}}/></AgentTurnChangeSummary>);
 expect(host.querySelector("header strong")?.textContent).toBe("Edited 2 files");
 await act(async()=>host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")!.click());
 expect(first).toHaveBeenCalledTimes(1);expect(second).not.toHaveBeenCalled();
 expect(host.textContent).not.toContain("部分修改已撤销");
 expect(host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")!.disabled).toBe(true);
});
it("retains every active contribution's guard when their Undo owner is shared",async()=>{
 const first=vi.fn().mockResolvedValue(true),second=vi.fn().mockResolvedValue(true);
 const host=await mount(<AgentTurnChangeSummary><AgentChangeContribution ownerIdentity="original-proposal" ariaLabel="first" items={[item("first")]} undo={{disabled:true,run:first}}/>
  <AgentChangeContribution ownerIdentity="original-proposal" ariaLabel="second" items={[item("second")]} undo={{disabled:false,run:second}}/></AgentTurnChangeSummary>);
 const button=host.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")!;
 expect(button.disabled).toBe(true);await act(async()=>button.click());
 expect(first).not.toHaveBeenCalled();expect(second).not.toHaveBeenCalled();
});
it("keeps different turns separate and deduplicates resource entries only within their owner",async()=>{
 const host=await mount(<><AgentTurnChangeSummary><AgentChangeContribution ariaLabel="a" items={[item("same")]}/><AgentChangeContribution ariaLabel="b" items={[item("same")]}/></AgentTurnChangeSummary><AgentTurnChangeSummary><AgentChangeContribution ariaLabel="c" items={[item("same")]}/></AgentTurnChangeSummary></>);
 expect(host.querySelectorAll(".writing-agent-inline-diff-card")).toHaveLength(2);
 expect([...host.querySelectorAll("header strong")].map(node=>node.textContent)).toEqual(["Edited file same","Edited file same"]);
});
it("the production transcript combines file, note and structured adapters into one turn card and Undo",async()=>{
 const scope={conversationId:"conversation",manuscriptId:"manuscript",requestId:"request",contextSnapshotId:"snapshot"};
 const binding={scope,turnId:"one",leaseId:"lease",manifestHash:"a".repeat(64)};
 const change={path:"main.tex",kind:"modified",before:null,after:{sha256:"b".repeat(64),byteSize:3,executable:false},hasEditorBuffer:false};
 mocks.files.latest.mockResolvedValue({proposalId:"file-change",review:{schemaVersion:1,binding,changes:[change]}});
 mocks.files.readFile.mockResolvedValue({file:{before:null,after:{kind:"text",text:"new"}}});
 const note={proposalId:"note-change",generation:1,state:"pending",persistenceVersion:2,baselineMatches:true,pages:[{pageId:"note",title:"笔记",kind:"update",before:"old",after:"new",expectedRevision:1}]};
 mocks.notes.review.mockResolvedValue(note);mocks.notes.decide.mockImplementation(async request=>{mocks.notes.review.mockResolvedValue({...note,state:"reject"});return request;});
 const structured={id:"portrait",title:"文献画像",kind:"portrait",sourceUri:"source://library/paper/portrait",revision:1,content:{claim:"new"},reviewBaseline:null,pendingFieldIds:["claim"],reviewedFieldIds:[]};
 mocks.structured.get.mockResolvedValue(structured);mocks.structured.getVersion.mockResolvedValue(structured);mocks.structured.review.mockResolvedValue({...structured,pendingFieldIds:[],revision:2});
 const results=[{path:"source://notes/note",name:"笔记",status:"applied",message:"saved",changeId:"note-change"},{path:"source://library/paper/portrait",name:"文献画像",status:"applied",message:"saved",changeId:"portrait-change",structuredResource:{resourceId:"portrait",revision:1}}];
 const messages:WritingAgentMessage[]=[{id:"user",role:"user",turnId:"one",content:"修改"},...results.map((target,index)=>({id:`result-${index}`,turnId:"one",role:"assistant" as const,kind:"tool-result" as const,toolName:"research_apply_patch",conversationId:"conversation",content:JSON.stringify({targets:[target]})})),{id:"final",turnId:"one",role:"assistant",content:"已完成",turnFinished:true}];
 const undoFile=vi.fn().mockResolvedValue({kind:"state",state:{status:"applied"}});
 const host=await mount(<WritingAgentTranscript messages={messages} isRunning={false} turnAttachment={()=><WritingWorkspaceChangeSummary scope={scope} disabled={false} undoProposal={undoFile}/>}/>);
 expect(host.querySelectorAll(".writing-agent-inline-diff-card")).toHaveLength(1);
 const card=host.querySelector(".writing-agent-inline-diff-card")!;
 expect(card.querySelector("header strong")?.textContent).toBe("Edited 3 files");
 expect(card.textContent).toContain("main.tex");expect(card.textContent).toContain("笔记");expect(card.textContent).toContain("文献画像");
 expect(card.closest("article")?.getAttribute("data-agent-message-id")).toBe("final");
 await act(async()=>card.querySelector<HTMLButtonElement>(".writing-agent-inline-diff-undo")!.click());
 expect(undoFile).toHaveBeenCalledTimes(1);expect(mocks.notes.decide).toHaveBeenCalledTimes(1);expect(mocks.structured.review).toHaveBeenCalledTimes(1);
});
it("keeps uncovered patch receipts while a file summary loads and after it confirms a partial inventory",async()=>{
 const scope={conversationId:"conversation",manuscriptId:"manuscript",requestId:"request",contextSnapshotId:"snapshot"};
 const binding={scope,turnId:"one",leaseId:"lease",manifestHash:"a".repeat(64)};
 const change={path:"main.tex",kind:"modified",before:null,after:{sha256:"b".repeat(64),byteSize:3,executable:false},hasEditorBuffer:false};
 let complete!:(value:unknown)=>void;
 mocks.files.latest.mockReturnValue(new Promise(resolve=>{complete=resolve;}));
 mocks.files.readFile.mockResolvedValue({file:{before:null,after:{kind:"text",text:"new"}}});
 const messages:WritingAgentMessage[]=[{id:"user",role:"user",turnId:"one",content:"修改"},
  {id:"patch",turnId:"one",role:"assistant",kind:"tool-result",toolName:"research_apply_patch",conversationId:"conversation",
   content:JSON.stringify({targets:[{path:"source://writings/manuscript/main.tex",name:"main.tex",status:"applied",message:"done"},
    {path:"source://writings/manuscript/other.tex",name:"other.tex",status:"applied",message:"done"}]})},
  {id:"final",turnId:"one",role:"assistant",content:"完成",turnFinished:true}];
 function Harness(){
  const [paths,setPaths]=useState<readonly string[]>([]);
  return <WritingAgentTranscript messages={messages} isRunning={false}
   turnAttachment={()=> <WritingWorkspaceChangeSummary scope={scope} disabled={false} onAvailablePathsChange={setPaths}/>}
   turnAttachmentPaths={()=>new Set(paths.map(path=>`source://writings/manuscript/${path}`))}/>;
 }
 const host=await mount(<Harness/>);
 expect(host.textContent).toContain("main.tex");expect(host.textContent).toContain("other.tex");
 expect(host.textContent).not.toContain("正在读取文件变更");
 await act(async()=>complete({proposalId:"file-change",review:{schemaVersion:1,binding,changes:[change]}}));
 await vi.waitFor(()=>expect(host.querySelector('[aria-label="消息中的文件改动"]')).not.toBeNull());
 expect(host.querySelectorAll(".writing-agent-inline-diff-card")).toHaveLength(1);
 expect(host.textContent).toContain("other.tex");
 const fallback=host.querySelector('[data-agent-message-id="final"] .writing-agent-message-bubble > ul');
 expect(fallback?.textContent).toContain("other.tex");expect(fallback?.textContent).not.toContain("main.tex");
});
