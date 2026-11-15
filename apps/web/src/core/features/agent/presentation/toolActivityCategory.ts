import type {ToolActivityRow} from "./toolActivityModel";
export const toolActivityCategoryOrder=["tool","edit","search","read","command"] as const;
export type ToolActivityCategory=typeof toolActivityCategoryOrder[number];
export function toolActivityCategory(row:ToolActivityRow):ToolActivityCategory {
 const name=(row.call?.name??row.result?.name??"").replace(/^research_/u,"");
 if(["shell","exec_command"].includes(name))return "command";
 if(/^(apply_patch|deliver_files)$/u.test(name))return "edit";
 if(/^(search(?:_.*)?|find_sources)$/u.test(name))return "search";
 if(/^(read_file|read_source|read_workspace|review_workspace|list_files|list_workspace|list_sources)$/u.test(name))return "read";
 return "tool";
}
/** Summary order also selects the headline icon; ties retain execution order. */
export function orderedToolActivities(rows:readonly ToolActivityRow[]):ToolActivityRow[] {
 return [...rows].sort((a,b)=>toolActivityCategoryOrder.indexOf(toolActivityCategory(a))-toolActivityCategoryOrder.indexOf(toolActivityCategory(b)));
}
