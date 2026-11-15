import { AgentModelControl, type AgentModelControlProps } from "../../../public/agentPresentation/AgentModelControl";
export function WritingAgentModelControl({route="writing_agent",label="选择 AI 写作模型",...props}: Omit<AgentModelControlProps, "route" | "label"> & {readonly route?:string;readonly label?:string}): React.JSX.Element {
  return <AgentModelControl {...props} route={route} label={label} />;
}
