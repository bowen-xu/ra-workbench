import {useEffect,useId,useState,type MouseEvent} from "react";
import {CodeXml} from "lucide-react";
import {ControlHint} from "../../components/ControlHint";
import {useDeveloperMode} from "../settings/DeveloperMode";
import "./AgentMessageData.css";

/** Keep the trigger and selectable data in separate host layout slots. */
export function useAgentInlineData(data:unknown,className="",preserveDisclosure?:()=>void) {
 const {enabled}=useDeveloperMode();
 const [open,setOpen]=useState(false);
 const id=useId();
 useEffect(()=>{if(!enabled)setOpen(false);},[enabled]);
 function toggle(event:MouseEvent<HTMLButtonElement>):void {
  event.preventDefault();event.stopPropagation();
  setOpen(value=>!value);
  preserveDisclosure?.();
 }
 const trigger=enabled?<ControlHint content="查看原始数据"><button type="button" className={`agent-message-data-trigger ${className}`}
  aria-label="查看原始数据" aria-expanded={open} aria-controls={id}
  onClick={toggle}>
  <CodeXml aria-hidden="true" size={13}/></button></ControlHint>:null;
 const content=enabled&&open?<pre id={id} className="agent-message-data-inline" aria-label="消息原始数据" tabIndex={0}>{JSON.stringify(data,null,2)}</pre>:null;
 return {trigger,content};
}
