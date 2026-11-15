import type {WritingDiffDemoHistory} from "./WritingDiffDemoHistory";
import type {WritingSubmissionAdmission} from "../../../core/features/agent/writingSubmissionController";
/** Event data only: exercise the production transcript and its real scroll/copy controls. */
export function seedDemoToolHistory(history:WritingDiffDemoHistory, admission:WritingSubmissionAdmission, running:boolean): (() => void) | undefined {
 const extraLines=Array.from({length:18},(_,i)=>[`-const value${i+1} = before${i+1};`,`+const value${i+1} = after${i+1};`]).flat();
 for(let index=0;index<20;index++) {
  const patch=index===1||index===8,command=index===0||index===19,skill=index===10,search=!patch&&!command&&!skill&&index%3===0;
  const path=index===8?"conflict.md":"src/preview.ts";
  const name=command?"research_exec_command":patch?"research_apply_patch":skill?"research_load_skill":search?"research_search_sources":"research_read_file";
  const args=command?{cmd:index===0?"printf 'preview tool output\\n'":"cat preview-output.log"}
   :patch?{patch:`*** Begin Patch\n*** Update File: ${path}\n@@\n const retained = true;\n-const output = oldText;\n+const output = newText;\n return output;\n@@\n function finish() {\n-return oldStatus;\n+return newStatus;\n }${index===1?"\n@@\n"+extraLines.join("\n"):""}\n*** End Patch`}
   :skill?{identity:{environment_id:"preview",source_id:".agents/skills/mock-research/SKILL.md",revision:"fixture"}}
   :search?{query:`证据 ${index+1}`,...(index===3?{pathGlob:"资料/*.md"}:{})}
   :{path:`资料/文献-${index+1}.md`,...(index===4?{startChar:12,maxChars:100}:{})};
  const output=command?{exitCode:0,stdout:{head:index===0?"preview tool output\n":Array.from({length:50},(_,line)=>`模拟第 ${line+1} 行输出`).join("\n"),tail:"",omittedBytes:0},stderr:{head:"",tail:"",omittedBytes:0}}
   :index===1?{status:"complete",targets:[{path,name:"preview.ts",status:"applied",message:"模拟修改完成"}],fileDiffs:[{path,hunks:[{oldStart:28,newStart:28,lines:[" const retained = true;","-const output = oldText;","+const output = newText;"," return output;"]},{oldStart:60,newStart:60,lines:[" function finish() {","-return oldStatus;","+return newStatus;"," }"]},{oldStart:100,newStart:100,lines:extraLines}]}]}
   :index===8?{error:"patch_conflict",message:"模拟补丁上下文不匹配，修改未应用"}
   :index===7?{error:"file_not_found",message:"模拟读取失败：找不到文件"}
   :skill?{skill:{name:"模拟研究",body:"模拟技能正文"}}
   :{status:"ok",message:"模拟读取完成",...(index===2?{startChar:0,nextChar:null,startLine:1,endLine:20}:index===4?{startChar:12,nextChar:112,startLine:3,endLine:8}:{})};
  history.toolStep(admission,{name,arguments:args,output,...(index===7||index===8?{isError:true}:{})});
 }
 if(running){const itemId=crypto.randomUUID(),callId=crypto.randomUUID(),turnId=admission.turnId;
  history.feedbackEvents(admission,[{type:"phaseChanged",turnId,phase:"runningTools"},
   {type:"itemStarted",turnId,itemId,kind:{type:"toolCall",callId,name:"research_apply_patch"}},
   {type:"textDelta",turnId,itemId,text:JSON.stringify({path:"当前正在处理.md"})}]);
  return () => { const resultId=crypto.randomUUID(); history.feedbackEvents(admission,[
   {type:"itemCompleted",turnId,itemId},
   {type:"itemStarted",turnId,itemId:resultId,kind:{type:"toolResult",callId}},
   {type:"textDelta",turnId,itemId:resultId,text:JSON.stringify({status:"complete",message:"模拟工具执行完成"})},
   {type:"itemCompleted",turnId,itemId:resultId},{type:"phaseChanged",turnId,phase:"runningModel"},
  ]); };
 }
 return undefined;
}
