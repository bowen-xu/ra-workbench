export type NoteHistoryDirection="undo"|"redo";
export type { NoteHistoryCommand } from "../../../contracts/noteResources";
import type { NoteHistoryCommand } from "../../../contracts/noteResources";
/** Shared note command ordering. Text entries delegate to the original native
 * history group; this owns neither document snapshots nor a second text history.
 * Lists, page actions and notifications must enter through the same command. */
export class NoteCommandHistory {
  private done:NoteHistoryCommand[]=[];
  private undone:NoteHistoryCommand[]=[];
  private owners=new Map<object,{readonly closeGroup:()=>void;readonly root?:()=>HTMLElement}>();
  private running:NoteHistoryCommand|undefined;
  private busy=false;
  register(owner:object,closeGroup:()=>void,root?:()=>HTMLElement):()=>void {
    this.owners.set(owner,{closeGroup,...(root?{root}:{})});
    return()=>{queueMicrotask(()=>{
      // PM reconfiguration recreates view hooks for the same owner/state.
      if(this.owners.get(owner)?.closeGroup!==closeGroup)return;
      this.owners.delete(owner);this.done=this.done.filter(command=>command.owner!==owner);this.undone=this.undone.filter(command=>command.owner!==owner);
    });};
  }
  closeGroups():void {for(const owner of this.owners.values())owner.closeGroup();}
  private top(direction:NoteHistoryDirection,owner?:object|null):NoteHistoryCommand|undefined {
    const entries=direction==="undo"?this.done:this.undone;
    for(let i=entries.length-1;i>=0;i--){const command=entries[i]!;if(!command.owner||owner===undefined||command.owner===owner)return command;}
    return undefined;
  }
  record(command:NoteHistoryCommand):void {
    if(this.running)return;
    this.done=this.done.filter(previous=>previous.id!==command.id);
    this.done.push(command);
    let excess=this.done.filter(value=>value.owner===command.owner).length-100;
    this.done=this.done.filter(value=>value.owner!==command.owner||excess--<=0);
    // A note's new input invalidates its own native redo and lifecycle redo;
    // another open note's native branch still belongs to that editor.
    this.undone=this.undone.filter(value=>value.owner&&value.owner!==command.owner);
  }
  permits(owner:object,direction:NoteHistoryDirection):boolean {
    const top=this.top(direction,owner);
    return top?.owner===owner&&(!this.busy||top===this.running);
  }
  nativeReplayed(owner:object,direction:NoteHistoryDirection):void {
    if(this.running)return;
    const from=direction==="undo"?this.done:this.undone,to=direction==="undo"?this.undone:this.done;
    const command=this.top(direction,owner);
    if(command?.owner===owner){from.splice(from.indexOf(command),1);to.push(command);}
  }
  async run(direction:NoteHistoryDirection,id?:string,target?:Node):Promise<boolean> {
    if(this.busy)return false;
    const owner=target?[...this.owners].find(([,value])=>value.root?.().contains(target))?.[0]??null:undefined;
    const command=this.top(direction,owner);
    if(!command)return false;
    if(id&&command.id!==id)throw new Error("note_history_order: 请先撤销后续笔记操作，再撤销这次删除。");
    this.busy=true;this.running=command;
    try {
      if(await command[direction]()===false)return false;
      const from=direction==="undo"?this.done:this.undone,to=direction==="undo"?this.undone:this.done;
      // A deleted editor may unmount while a lifecycle command is running.
      const index=from.indexOf(command);if(index>=0)from.splice(index,1);
      to.push(command);return true;
    }finally{this.running=undefined;this.busy=false;}
  }
}
