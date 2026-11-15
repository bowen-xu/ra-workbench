import type {ReactNode} from "react";
import { AgentMessageCopyButton } from "../../../public/agentPresentation/AgentMessageCopyButton";
/** The owning history row scrolls the complete card, including its header. */
export function ToolContentCard({title,copy,label,children,className=""}:{readonly title:ReactNode;readonly copy:string;readonly label:string;readonly children:ReactNode;readonly className?:string}) {
 return <section className={`tool-content-card ${className}`}><header>{title}<AgentMessageCopyButton value={copy} label={label} className="tool-content-copy"/></header>{children}</section>;
}
