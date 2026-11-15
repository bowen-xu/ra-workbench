import {getAppRuntime} from "../../../runtime/appRuntime";
import {save} from "../../../runtime/platformDialog";
import {invoke} from "../../../runtime/platformCore";

/** Recovery files must work even when the application database cannot write.
 * Preview downloads contain mock data and use the same browser file path. */
export async function saveNoteRecoveryFile(name:string, value:unknown, options:{readonly title?:string} = {}):Promise<"saved"|"download-started"|"cancelled"> {
  const content=JSON.stringify(value,null,2);
  if(getAppRuntime().mode==="desktop") {
    const destination=await save({title:options.title ?? "导出笔记恢复材料",defaultPath:name,filters:[{name:"JSON",extensions:["json"]}]});
    if(!destination)return "cancelled";
    try {await invoke("export_note_recovery_file",{destination,content});}
    catch(reason){
      if(reason&&typeof reason==="object"&&"message" in reason&&typeof reason.message==="string") {
        const code="code" in reason?String(reason.code):"NOTE_RECOVERY_EXPORT_FAILED";
        const osError="osError" in reason&&typeof reason.osError==="number"?`; osError=${reason.osError}`:"";
        throw Object.assign(new Error(`${code}：${reason.message}${osError}`),{code});
      }
      throw reason;
    }
    return "saved";
  }
  const url=URL.createObjectURL(new Blob([content],{type:"application/json;charset=utf-8"}));
  try {const anchor=document.createElement("a");anchor.href=url;anchor.download=name;anchor.click();}
  finally {setTimeout(()=>URL.revokeObjectURL(url),1000);}
  return "download-started";
}
