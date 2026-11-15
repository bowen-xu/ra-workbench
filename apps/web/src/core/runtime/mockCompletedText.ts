import {decodeAgentEvent,type AgentEvent} from "@research-assistant/core/agent";

const key=(event:AgentEvent,itemId:string)=>JSON.stringify([event.conversationId,event.payload.turnId,itemId]);
/** Persistence adapter only: memory/live gateways retain the normal event contract. */
export function packCompletedText(value:unknown):unknown {
  if(!Array.isArray(value)||!value.length||!value.every(row=>row?.schemaVersion===1&&row.payload?.type))return undefined;
  const events=value as AgentEvent[];
  const completed=new Set(events.flatMap(event=>event.payload.type==="itemCompleted"?[key(event,event.payload.itemId)]:[]));
  const messages=new Map<string,string>();
  const entries=events.map(event=>{
    if(event.payload.type!=="textDelta")return event;
    const id=key(event,event.payload.itemId);if(!completed.has(id))return event;
    const body=messages.get(id)??"";messages.set(id,body+event.payload.text);
    const {text,...payload}=event.payload;
    return {...event,payload:{...payload,type:"storedTextSlice",offset:body.length,length:text.length}};
  });
  return messages.size?{mockType:"agentEvents",entries,messages:[...messages]}:undefined;
}
export function unpackCompletedText(value:{entries:unknown[];messages:unknown}):AgentEvent[] {
  if(!Array.isArray(value.messages)||!value.messages.every(row=>Array.isArray(row)&&row.length===2&&typeof row[0]==="string"&&typeof row[1]==="string"))throw new Error("预览消息正文格式错误");
  const messages=new Map<string,string>(value.messages);
  return value.entries.map(row=>{
    const event=row as AgentEvent;
    const payload=event.payload as unknown as {type:string;itemId:string;turnId:string;offset:number;length:number};
    if(payload.type!=="storedTextSlice")return decodeAgentEvent(row);
    const body=messages.get(key(event,payload.itemId));
    if(body===undefined||!Number.isSafeInteger(payload.offset)||!Number.isSafeInteger(payload.length)||payload.offset<0||payload.length<0||payload.offset+payload.length>body.length)throw new Error("预览消息正文索引无效");
    return decodeAgentEvent({...event,payload:{type:"textDelta",turnId:payload.turnId,itemId:payload.itemId,text:body.slice(payload.offset,payload.offset+payload.length)}});
  });
}
