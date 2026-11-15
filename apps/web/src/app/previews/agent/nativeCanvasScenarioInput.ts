import { insertAgentReference, serializeAgentSourceReferences, type AgentUserInput } from "@research-assistant/core/agent";
import { formatSourceLink } from "@research-assistant/core/source";
import { agentHostResourceGateway, agentMcpGateway, agentSkillCatalogGateway } from "../../../core/services/agentGateway";
import { canvasFixtureError, type NativeCanvasFixture, type NativeCanvasStart } from "./nativeCanvasScenarioTypes";

/** References are discovered through actual isolated production catalogs. */
export async function nativeCanvasScenarioInput(fixture: NativeCanvasFixture): Promise<NativeCanvasStart> {
  const text = "开始处理当前稿件", scenario = fixture.scenario;
  const base = { requestId: fixture.startRequestId, text, queuedTexts: [] };
  if (scenario === "longHistory") return {...base, text:"长历史第 1 轮：检查研究问题与证据。",
    queuedTexts:Array.from({length:59},(_,index)=>`长历史第 ${index+2} 轮：继续检查方法、证据与结论，保留 Markdown 格式。`)};
  if (scenario === "evidence") {
    const source = formatSourceLink({path:["writings",fixture.manuscriptId,...fixture.activePath.split("/")]});
    return {...base,userInput:[{type:"text",text,textElements:[]},
      {type:"annotation",source:`${source}:#line{2-1:2-26}`,title:"关键词定义",text:"\\keywords{research, agent}",comment:"请结合文章内容检查这些关键词。"},
      {type:"annotation",source:`${source}:#line{2-1:2-26}`,title:`${fixture.activePath} · 关键词`,text:"\\keywords{research, agent}",comment:""},
      {type:"annotation",source:`${source}:#line{1-1:1-23}`,title:`${fixture.activePath} · 文档类型`,text:"\\documentclass{article}",comment:""}]};
  }
  if (!["longMessage","resources","invalidSkill"].includes(scenario)) return base;
  const catalog = await agentSkillCatalogGateway.list(), skill = catalog.skills.find(value=>value.enabled&&value.sourceId.includes(fixture.fixtureId));
  if (!skill) throw canvasFixtureError("CANVAS_FIXTURE_SKILL_UNAVAILABLE");
  const reference = {type:"skill" as const,name:skill.name,identity:{environment_id:catalog.environmentId,source_id:skill.sourceId,revision:skill.revision}};
  if (scenario === "longMessage") {
    const resources=await agentHostResourceGateway({kind:"manuscript",resourceId:fixture.manuscriptId,conversationId:fixture.conversationId});
    const file=resources.find(value=>value.label===fixture.activePath&&value.input.type==="mention"&&value.input.target.kind==="file");
    if(!file)throw canvasFixtureError("CANVAS_FIXTURE_FILE_REFERENCE_UNAVAILABLE");
    const draft=insertAgentReference(insertAgentReference({text:"🧪 请结合以下引用检查稿件。\n",bindings:[]},reference),file.input);
    const long=draft.text+"\n"+Array.from({length:20},(_,index)=>`第 ${index+1} 项：检查研究问题、方法、证据和结论的对应关系，并保留引用来源。`).join("\n");
    const input=serializeAgentSourceReferences({...draft,text:long},{kind:"manuscript",resourceId:fixture.manuscriptId,conversationId:fixture.conversationId});
    return {...base,text:input.text,userInput:input.userInput};
  }
  const userInput:AgentUserInput[]=[{type:"text",text,textElements:[]},reference];
  if(scenario==="resources"){
    const tool=(await agentMcpGateway.list()).find(value=>value.enabled&&value.credentialConfigured&&value.approved);
    if(!tool)throw canvasFixtureError("CANVAS_FIXTURE_TOOL_REFERENCE_UNAVAILABLE");
    userInput.push({type:"mention",name:tool.serverId,target:{kind:"tool",serverId:tool.serverId,environmentId:tool.environmentId,revision:tool.revision}});
  }
  return {...base,userInput};
}
