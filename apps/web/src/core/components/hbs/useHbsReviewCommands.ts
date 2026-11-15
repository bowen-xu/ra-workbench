import {useEffect,useId,useRef,useState} from "react";
import type {HbsSession,HbsSessionState} from "@research-assistant/core/hbs";
import type {NoteDocumentV1} from "@research-assistant/core/notes";
import type {EditorReviewAction} from "../editor-diff/EditorReviewActions";
import {notify} from "../../services/notifications";

/** Feedback and explicit retry adapter only. The HBS session retains every
 * decision, queue, immutable request and receipt; this hook owns no writer. */
export function useHbsReviewCommands(session:HbsSession<NoteDocumentV1>, state:HbsSessionState<NoteDocumentV1>, options:{
  readonly onRetrySave?: (()=>Promise<void>) | undefined;
  readonly retryRequired?: boolean;
}) {
  const id=useId(),attempt=useRef<{label:string;failed:boolean}|undefined>(undefined),retrying=useRef(false);
  const [sequence,redraw]=useState(0);
  const failure=(current:HbsSessionState<NoteDocumentV1>)=>current.preservationError
    ?? (/^MODEL_(RESULT_UNKNOWN|REVIEW_FAILED|EDITOR_INSTALL_FAILED|EDITOR_NOT_READY)/u.test(current.message)?current.message:undefined);
  function showFailure(reason:string) {
    const current=attempt.current;if(!current||current.failed)return;
    current.failed=true;
    const pending=session.getSnapshot();
    const recorded=options.retryRequired||pending.pending||!!pending.preservationError&&pending.queued>0;
    notify.error(`${current.label} 尚未保存：${recorded?"选择已记录，待审 Diff 暂时保留。可再次点击 Keep／Keep all 核对并重试原请求。":"操作未完成，正文已保留，请核对后重试。"}\n${reason}`,{id:`hbs-review:${id}`});
  }
  useEffect(()=>{
    if(!attempt.current||retrying.current)return;
    const reason=failure(state);
    if(reason)showFailure(reason);
    else if(!state.pending&&!state.queued&&!options.retryRequired){
      if(attempt.current.failed)notify.success("上次审阅请求已确认。",{id:`hbs-review:${id}`});
      attempt.current=undefined;
    }
  },[state,sequence,options.retryRequired]);
  function run(action:EditorReviewAction,apply:()=>void):void {
    const current=session.getSnapshot();
    const retry=action==="keep"&&(options.retryRequired||current.pending&&!!failure(current)||!!current.preservationError&&current.queued>0);
    if(retrying.current)return;
    if(action==="keep"||action==="undo")attempt.current={label:retry?attempt.current?.label??"上次操作":action==="keep"?"Keep":"Undo",failed:false};
    redraw(value=>value+1);
    if(!retry){
      try {apply();}
      catch(reason){showFailure(reason instanceof Error?reason.message:String(reason));}
      return;
    }
    retrying.current=true;
    void (async()=>{
      try {
        await (options.onRetrySave?options.onRetrySave():session.save());
        const next=session.getSnapshot(),reason=failure(next);
        if(reason)showFailure(reason);
        else if(!next.pending&&next.conflicts.length){
          notify.info("原请求已核对；版本已变化，请在正文内重新核对冲突。",{id:`hbs-review:${id}`});
          attempt.current=undefined;
        }
        else if(!next.pending&&!next.queued){
          notify.success("原请求已核对；若仍有待审内容，可继续审阅。",{id:`hbs-review:${id}`});
          attempt.current=undefined;
        }
      }catch(reason){showFailure(reason instanceof Error?reason.message:String(reason));}
      finally {retrying.current=false;redraw(value=>value+1);}
    })();
  }
  return run;
}
