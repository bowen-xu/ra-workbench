import { MoreHorizontal } from "lucide-react";
import { EditorDiffControls } from "../editor-diff/EditorDiffControls";
import { ActionMenu, ActionMenuItem } from "../ActionMenu";
import { ControlHint } from "../ControlHint";
export function StructuredReviewControls({count,active,busy,onMove,onResolve,
  onKeepUnreviewed,metadataActive}:{
  readonly count:number;readonly active:number;readonly busy:boolean;
  readonly onMove:(direction:"previous"|"next")=>void;
  readonly onResolve:(action:"keep"|"undo",scope:"current"|"all")=>void;
  readonly onKeepUnreviewed:()=>void;readonly metadataActive:boolean;
}){
  return <div className={`structured-resource-bulk${count?" has-diff":""}`} role="toolbar" aria-label="结构化阅读记录修改审阅">
    <EditorDiffControls count={count} active={active} disabled={busy} showBulk={false} onMove={onMove} onResolve={onResolve}/>
    {count>0&&<ActionMenu label="更多阅读审阅操作" className="structured-review-more" trigger={<MoreHorizontal size={15}/>} portal placement="bottom-end"
      disabled={busy} hint triggerClassName="reading-icon-action" contentClassName="structured-review-menu">
      {metadataActive&&<ControlHint tooltipClassName="structured-review-hint" content="当前变更涉及模板或来源范围信息，使用当前变更操作即可审阅。"><ActionMenuItem disabled>模板／来源</ActionMenuItem></ControlHint>}
      <ControlHint tooltipClassName="structured-review-hint" content="撤销所有尚未接受的差异，恢复修改前的内容；本次已 Keep 的内容保留。"><ActionMenuItem onClick={()=>onResolve("undo","all")}>Undo all</ActionMenuItem></ControlHint>
      <ControlHint tooltipClassName="structured-review-hint" content="接受所有尚未处理的差异，包括原来已人工校验的字段；Keep 不代表已核验事实或引用。"><ActionMenuItem onClick={()=>onResolve("keep","all")}>Keep all</ActionMenuItem></ControlHint>
      <ControlHint tooltipClassName="structured-review-hint" content="仅接受修改前未人工校验字段的差异，保留原来已人工校验字段的待审修改。"><ActionMenuItem onClick={onKeepUnreviewed}>Keep 未校验</ActionMenuItem></ControlHint>
    </ActionMenu>}
  </div>;
}
