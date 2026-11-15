import { AgentResourceName } from "../../../public/agentContext/AgentResourceName";
import type {ToolActivityRow} from "./toolActivityModel";
import {toolReadDescription} from "./toolReadActivity";
export function ToolReadTitle({row}:{readonly row:ToolActivityRow}){
 const read=toolReadDescription(row);
 if(!read)return null;
 return <>已读取 <AgentResourceName path={read.path??read.identity??read.name} name={read.name} className="tool-read-file-name"/>{read.range}</>;
}
