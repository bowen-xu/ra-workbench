import {closeHistory,isHistoryTransaction,undo} from "@tiptap/pm/history";
import {Plugin,PluginKey,type Transaction} from "@tiptap/pm/state";
import {Step,StepResult,type Mappable} from "@tiptap/pm/transform";
import type {Node} from "@tiptap/pm/model";
import type {NoteEditorInstance} from "./noteEditorSchema";
import type {NoteCommandHistory,NoteHistoryDirection} from "./NoteCommandHistory";

/** A position-free group identity follows native PM grouping/mapping. Undo
 * probes are pure; publishing and command ordering happen only in view.update. */
class CommandIdentity extends Step {
  constructor(readonly id:string,readonly direction:NoteHistoryDirection="redo"){super();}
  apply(doc:Node):StepResult{return StepResult.ok(doc);}
  invert():Step{return new CommandIdentity(this.id,this.direction==="undo"?"redo":"undo");}
  map(_mapping:Mappable):Step{return this;}
  toJSON():unknown{return{stepType:"noteCommandIdentityV1",id:this.id,direction:this.direction};}
  static override fromJSON(_schema:unknown,value:{id:string;direction:NoteHistoryDirection}):Step{return new CommandIdentity(value.id,value.direction);}
}
Step.jsonID("noteCommandIdentityV1",CommandIdentity);
export function noteCommandHistoryPlugin(history:NoteCommandHistory,getEditor:()=>NoteEditorInstance,canReplay:()=>boolean):Plugin {
  const owner={},key=new PluginKey<Transaction|undefined>("note-command-history");
  return new Plugin<Transaction|undefined>({key,
    state:{init:()=>undefined,apply:(tr,previous)=>tr.getMeta(key) as Transaction|undefined??previous},
    filterTransaction(tr){
      if(!isHistoryTransaction(tr))return true;
      const direction=tr.steps.find(step=>step instanceof CommandIdentity)?.direction;
      return canReplay()&&(!direction||history.permits(owner,direction));
    },
    appendTransaction(transactions,_before,after){
      const first=transactions[0],origin=first?.getMeta("appendedTransaction") as Transaction|undefined??first;
      if(!origin||key.getState(after)===origin)return null;
      if(isHistoryTransaction(origin))return after.tr.setMeta(key,origin);
      if(origin.getMeta("addToHistory")===false||!transactions.some(tr=>tr.docChanged))return null;
      return after.tr.step(new CommandIdentity(crypto.randomUUID())).setMeta(key,origin);
    },
    view(view){
      let previous=key.getState(view.state);
      const stop=history.register(owner,()=>view.dispatch(closeHistory(view.state.tr)),()=>view.dom);
      return{destroy:stop,update(){
        const current=key.getState(view.state);if(!current||current===previous)return;previous=current;
        if(isHistoryTransaction(current)){
          const direction=current.steps.find(step=>step instanceof CommandIdentity)?.direction;
          if(direction)history.nativeReplayed(owner,direction);return;
        }
        let id:string|undefined;
        // Last inverse identity belongs to the oldest input in this native group.
        undo(view.state,tr=>{id=tr.steps.filter(step=>step instanceof CommandIdentity).at(-1)?.id;});
        if(id)history.record({id,owner,undo:()=>canReplay()&&getEditor().undo(),redo:()=>canReplay()&&getEditor().redo()});
      }};
    },
  });
}
