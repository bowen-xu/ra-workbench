export const OPEN_AGENT_CONVERSATION = "research-assistant:open-agent-conversation";
export interface OpenAgentConversation { readonly id:string; readonly resolve:()=>void; readonly reject:(reason:unknown)=>void }
export function openAgentConversation(id:string):Promise<void> {
  return new Promise((resolve,reject) => {
    const event=new CustomEvent<OpenAgentConversation>(OPEN_AGENT_CONVERSATION,{detail:{id,resolve,reject},cancelable:true});
    if (window.dispatchEvent(event)) reject(new Error("当前界面未接入对话导航。"));
  });
}
