import { decodeAgentUserInput, sameAgentSkillIdentity, type AgentUserInput, type AgentSubmissionSkill, type AgentContextText, type AgentSkillInstructions, type AgentPickerResource, type AgentResourceScope } from "@research-assistant/core/agent";
import type { AgentResourceGateways } from "./agentResourceGateways";
import type {AgentSkillCatalogGateway} from "@research-assistant/data/agent";
import type {agentMcpGateway} from "../../services/agentGateway";
/** Mock native discovery, loading and permission checks. UI and submission
 * controllers use the same ports as production; no preview-only resource UI. */
export class MockAgentResourceRuntime {
 enabled=true;approved=true;revision="1";
 readonly name="模拟写作技能";
 readonly sourceId=".agents/skills/academic-writing/SKILL.md";
 private readonly skillContents="使用参考资料和只读 scripts/run.sh；脚本通过现有 shell 工具执行，缺少依赖时报告错误。";
 constructor(private readonly files:(scope:AgentResourceScope)=>Promise<readonly AgentPickerResource[]>=async()=>[]){}
 readonly gateways:AgentResourceGateways & {readonly skills: AgentSkillCatalogGateway; readonly tools: typeof agentMcpGateway}={
  skills:{list:async()=>({schemaVersion:1,environmentId:"application",errors:[],skills:[{sourceId:this.sourceId,name:this.name,description:"完整模拟技能：按需读取与脚本执行",revision:this.revision,allowImplicitInvocation:true,enabled:this.enabled}]}),setEnabled:async(id,revision,enabled)=>{if(id!==this.sourceId||revision!==this.revision)throw "conflict";this.enabled=enabled;return this.gateways.skills.list();}},
  tools:{list:async()=>[{serverId:"模拟文献检索",environmentId:"mock",revision:"1",enabled:true,credentialConfigured:true,approved:this.approved}],install:async ids=>{this.approved=ids.includes("模拟文献检索");},revoke:async()=>{this.approved=false;}},
  hostResources:scope=>this.files(scope),
  readSkillDocument:async identity=>{
   if(!sameAgentSkillIdentity(identity,{environment_id:"application",source_id:this.sourceId,revision:this.revision}))throw new Error("模拟技能版本已变化，请重新选择");
   return this.skillContents;
  },
 };
 async freeze(text:string,userInput:readonly AgentUserInput[]=[],_context:readonly AgentContextText[]=[],frozenCatalog?:readonly AgentSubmissionSkill[],initialToolsAllowed=this.approved){
  const input=decodeAgentUserInput(userInput,text),identity={environment_id:"application",source_id:this.sourceId,revision:this.revision};
  const skills:AgentSubmissionSkill[]=frozenCatalog?structuredClone([...frozenCatalog]):this.enabled?[{descriptor:{identity,name:this.name,description:"模拟写作技能",allow_implicit_invocation:true},explicit:false}]:[];
  const skillInstructions:AgentSkillInstructions[]=[];
  for(const item of input){if(item.type==="skill"){
    const skill=skills.find(skill=>sameAgentSkillIdentity(skill.descriptor.identity,item.identity)&&skill.descriptor.name===item.name);
    if(!this.enabled||!sameAgentSkillIdentity(identity,item.identity)||!skill)continue;
    skills[skills.indexOf(skill)]={...skill,explicit:true};if(!skillInstructions.some(prompt=>sameAgentSkillIdentity(prompt.identity,item.identity)))skillInstructions.push({identity:item.identity,name:item.name,contents:this.skillContents});
   }else if(item.type==="mention"){
    const target=item.target;
    if(target.kind==="tool"){if(!this.approved||!initialToolsAllowed||target.serverId!=="模拟文献检索"||target.environmentId!=="mock"||target.revision!=="1")continue;}

   }}
  return {userInput:input,skills,skillInstructions};
 }
}
