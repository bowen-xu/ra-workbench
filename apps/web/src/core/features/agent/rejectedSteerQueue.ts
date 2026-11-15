import type {AgentQueueEdit,AgentQueueState,AgentUserInput} from "@research-assistant/core/agent";

interface QueueOwner {
  getSnapshot():{readonly queue:AgentQueueState|null};
  edit(edit:AgentQueueEdit,acknowledgment?:()=>void,beforeSend?:undefined,expectedEpoch?:undefined,isAdmissionDisabled?:()=>boolean):Promise<boolean>;
  reload():Promise<void>;
}
interface QueueLease { readonly controller:QueueOwner;readonly release:()=>void }
/** A failed manual task must not automatically flush its deferred ordinary input. */
export async function pauseRejectedSteerQueue(lease:QueueLease|null):Promise<void>{
  if(!lease)return;
  try{
    const queue=lease.controller.getSnapshot().queue;
    if(queue?.inputs.length&&!queue.paused)await lease.controller.edit({type:"pause"});
  }finally{lease.release();}
}
export async function recoverRejectedSteer(lease:QueueLease|null,request:Parameters<typeof retainRejectedSteer>[1],acknowledge:()=>void,isAdmissionDisabled?:()=>boolean):Promise<boolean>{
  if(!lease)return false;
  try{
    if(!lease.controller.getSnapshot().queue)await lease.controller.reload();
    return await retainRejectedSteer(lease.controller,request,acknowledge,isAdmissionDisabled);
  }finally{lease.release();}
}
export async function prioritizeRejectedSteerLease(lease:QueueLease|null,inputId:string):Promise<boolean>{
  if(!lease)return false;
  try{
    if(!lease.controller.getSnapshot().queue)await lease.controller.reload();
    return await prioritizeRejectedSteer(lease.controller,inputId);
  }finally{lease.release();}
}
/** Only definitive rejection may transfer input to another turn. Queue receipts
 * continue to own unknown edits; a failed edit never clears the original draft. */
export async function retainRejectedSteer(owner:QueueOwner|null,request:{readonly inputId:string;readonly text:string;readonly userInput?:readonly AgentUserInput[]},acknowledge:()=>void,isAdmissionDisabled?:()=>boolean):Promise<boolean>{
  if(!owner)return false;
  const edit:AgentQueueEdit={type:"add",inputId:request.inputId,text:request.text,...(request.userInput?{userInput:request.userInput}:{})};
  const applied=isAdmissionDisabled?await owner.edit(edit,acknowledge,undefined,undefined,isAdmissionDisabled):await owner.edit(edit,acknowledge);
  if(!applied)return false;
  await prioritizeRejectedSteer(owner,request.inputId);
  return true;
}
export async function prioritizeRejectedSteer(owner:QueueOwner|null,inputId:string):Promise<boolean>{
  const queue=owner?.getSnapshot().queue;
  if(!owner||!queue||!queue.inputs.some(input=>input.inputId===inputId))return false;
  if(queue.inputs[0]?.inputId!==inputId
    &&!await owner.edit({type:"reorder",inputIds:[inputId,...queue.inputs.filter(input=>input.inputId!==inputId).map(input=>input.inputId)]}))return false;
  const prioritized=owner.getSnapshot().queue;
  const input=prioritized?.inputs.find(item=>item.inputId===inputId);
  if(!prioritized||!input)return false;
  // Explicit steering resumes only this input. The command owner still prevents
  // admission until compaction succeeds; other paused inputs remain paused.
  if(prioritized.paused||input.paused)return owner.edit({type:"resumeInput",inputId});
  return true;
}
