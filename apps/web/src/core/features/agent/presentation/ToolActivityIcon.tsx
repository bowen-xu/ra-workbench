import {BookOpen,Folder,Minus,Pencil,Search,SquareTerminal,Wrench,X} from "lucide-react";
import type {ToolActivityRow} from "./toolActivityModel";
import {toolPatchOutcome,toolShellInvocation} from "./toolActivityPresentation";
import {orderedToolActivities,toolActivityCategory} from "./toolActivityCategory";
const icons={tool:Wrench,edit:Pencil,search:Search,read:BookOpen,command:SquareTerminal};
function kind(row:ToolActivityRow):keyof typeof icons {
 return toolPatchOutcome(row)?"edit":toolShellInvocation(row)?"command":toolActivityCategory(row);
}
function activityIcon(row:ToolActivityRow,selected:keyof typeof icons) {
 const name=(row.call?.name??row.result?.name??"").replace(/^research_/u,"");
 return selected==="read"&&["list_files","list_workspace","list_sources"].includes(name)?Folder:icons[selected];
}
export function ToolActivityGroupIcon({rows}:{readonly rows:readonly ToolActivityRow[]}) {
 const first=orderedToolActivities(rows)[0],selected=first?toolActivityCategory(first):"tool",Icon=first?activityIcon(first,selected):icons.tool;
 return <span className="tool-activity-icon" data-tool-kind={selected}><Icon className="tool-history-group-kind" size={15} aria-hidden="true"/></span>;
}
export function ToolActivityIcon({row}:{readonly row:ToolActivityRow}) {
 const selected=kind(row),Icon=activityIcon(row,selected);
 const outcome=toolPatchOutcome(row);
 const Badge=outcome==="declined"?Minus:outcome==="failed"||row.status==="error"?X:row.status==="interrupted"?Minus:undefined;
 return <span className={`tool-activity-icon ${row.status}`} data-tool-kind={selected}><Icon className="tool-history-kind" size={15} aria-hidden="true"/>{Badge&&<Badge className="tool-history-status" size={9} aria-hidden="true"/>}</span>;
}
