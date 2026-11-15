/** Presentation order survives panel unmounts for the lifetime of its owner. */
export class PendingSteeringOrderStore {
  private readonly entries=new Map<string,{conversationId:string;ids:readonly string[]}>();
  read(key:string,conversationId:string):readonly string[]{const value=this.entries.get(key);return value?.conversationId===conversationId?value.ids:[];}
  commit(key:string,conversationId:string,ids:readonly string[]):void{
    if(!ids.length)return;
    const current=this.entries.get(key);
    const previous=current?.conversationId===conversationId?current.ids:[];
    // Views can temporarily observe different snapshots of the same owner;
    // absence from one view is not evidence that an input was resolved.
    const ordered=[...previous,...ids.filter(id=>!previous.includes(id))];
    this.entries.set(key,{conversationId,ids:ordered});
  }
  forgetConversation(conversationId:string):void{
    for(const [key,entry] of this.entries)if(entry.conversationId===conversationId)this.entries.delete(key);
  }
}
