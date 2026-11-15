import {useEffect,type RefObject} from "react";
import type {NoteCommandHistory} from "./NoteCommandHistory";
import {notify} from "../../../services/notifications";

/** Capture once at the active note surface, before the native editor keymap.
 * Text fields outside the note editor and owned dialogs keep their own undo. */
export function useNoteHistoryScope(history:NoteCommandHistory,root:RefObject<HTMLElement|null>,active=true):void {
  useEffect(()=>{
    if(!active)return;
    const element=root.current;if(!element)return;
    const handle=(event:KeyboardEvent|InputEvent)=>{
      if(event.defaultPrevented||event.isComposing)return;
      if(!element.getClientRects().length)return;
      const target=event.target instanceof HTMLElement?event.target:undefined;
      if(!target||!element.contains(target)&&target!==document.body)return;
      if(target.closest('input:not([type="checkbox"]):not([type="radio"]),textarea,[role="dialog"],[role="menu"]'))return;
      if(target.isContentEditable&&!target.closest('.note-editor'))return;
      const key=event instanceof KeyboardEvent?event.key.toLowerCase():undefined;
      const direction=event instanceof KeyboardEvent
        ? (event.metaKey||event.ctrlKey)&&!event.altKey?(key==="z"?(event.shiftKey?"redo":"undo"):key==="y"&&event.ctrlKey&&!event.metaKey?"redo":undefined):undefined
        : event.inputType==="historyUndo"?"undo":event.inputType==="historyRedo"?"redo":undefined;
      if(!direction)return;
      event.preventDefault();event.stopPropagation();
      void history.run(direction,undefined,target).catch(reason=>notify.error(reason instanceof Error?reason.message:String(reason)));
    };
    window.addEventListener("keydown",handle,true);window.addEventListener("beforeinput",handle,true);
    return()=>{window.removeEventListener("keydown",handle,true);window.removeEventListener("beforeinput",handle,true);};
  },[history,root,active]);
}
