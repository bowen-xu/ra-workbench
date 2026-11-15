import { useRef } from "react";
import { ControlHint } from "../../../components/ControlHint";
import { Switch } from "../../../components/Switch";
import { ChevronRight, FileText, LibraryBig, NotebookPen, type LucideIcon } from "lucide-react";
import { agentContextGroupParts, agentContextKey, canWriteAgentContext, isAgentContextGroup, type AgentContextPart, type AgentContextSelection } from "@research-assistant/core/agent";

export function ContextCatalogRow({item, selected, disabled, onChange}: {
  readonly item: AgentContextSelection; readonly selected?: AgentContextSelection;
  readonly disabled: boolean; readonly onChange: (item: AgentContextSelection | null) => void;
}): React.JSX.Element {
  const resource = item.resource;
  const box = useRef<HTMLInputElement>(null);
  if (!isAgentContextGroup(resource)) {
    const managed=resource.kind === "structuredObject" || resource.kind === "templateCatalog";
    const writable=selected && (selected.resource.kind === "structuredObject" || selected.resource.kind === "templateCatalog") && selected.resource.access === "readWrite";
    return <li className="context-catalog-item"><div className="context-catalog-part-row"><label><input type="checkbox" checked={!!selected} disabled={disabled} onChange={e => onChange(e.target.checked ? item : null)}/><CatalogLabel value={item.label}/></label>{managed && <label className="context-access-control"><span>{writable ? resource.kind === "templateCatalog" ? "可新建" : "读写" : "只读"}</span><Switch aria-label={`${item.label} 权限`} checked={!!writable} disabled={disabled || !selected} onChange={e=>onChange({...item,resource:{...resource,access:e.target.checked ? "readWrite" : "readOnly"}})}/></label>}</div></li>;
  }
  const group = resource;
  const saved = selected && isAgentContextGroup(selected.resource) ? selected.resource : undefined;
  const parts = saved ? agentContextGroupParts(resource, saved, true) : resource.parts.map(part => ({...part,selected:false}));
  const access = saved?.groupAccess ?? (!saved || saved.newMemberAccess === "readWrite" || parts.some(part => part.selected && part.access === "readWrite") ? "readWrite" : "readOnly");
  const readOnly = access === "readOnly";
  const notes = parts.filter(part => part.resource.kind === "note" && resource.kind === "paperGroup");
  const displayed = parts.filter(part => !notes.includes(part) || part.selected);
  const count = displayed.filter(part => part.selected).length;
  const all = count === displayed.length && (displayed.length > 0 || !!saved?.newMemberAccess);
  const Icon: LucideIcon = resource.kind === "paperGroup" ? LibraryBig : resource.kind === "manuscript" ? FileText : NotebookPen;
  function update(next: readonly AgentContextPart[], scope = saved?.newMemberAccess, ceiling = access): void {
    // Missing inventory members retain explicit exclusions if later restored.
    const missing = saved?.parts.filter(part => !next.some(value => agentContextKey(value.resource) === agentContextKey(part.resource))) ?? [];
    onChange(next.some(part => part.selected) || scope ? {...item, resource: {kind:group.kind,resourceId:group.resourceId,parts:[...next,...missing],...(scope ? {newMemberAccess:scope} : {}),...(ceiling ? {groupAccess:ceiling} : {})}} : null);
  }
  function replace(part: AgentContextPart, next: AgentContextPart): void {
    update(parts.map(value => value === part ? next : part.resource.kind === "paperNotes" && notes.includes(value) ? {...value,selected:false} : value));
  }
  return <li className="context-catalog-item" data-selected={!!selected}>
    <details open={!!selected}>
      <summary><ChevronRight className="context-catalog-chevron" size={14} aria-hidden="true"/><ControlHint content={group.kind === "noteGroup" ? undefined : "授权整个对象，新增成员自动纳入；可在下方排除单项"}><input ref={node => {box.current = node; if (node) node.indeterminate = (count > 0 || !!saved?.newMemberAccess) && !all;}} type="checkbox" aria-label={`使用 ${item.label}`} checked={all} disabled={disabled} onClick={e => e.stopPropagation()} onChange={e => {if (!e.target.checked) {onChange(null);return;} update((saved ? parts : resource.parts).map(part => ({...part,selected:!notes.includes(part),...(part.resource.kind === "paperNotes" ? {resource:{...part.resource,excludedNoteIds:[]}} : {})})),saved?.newMemberAccess ?? "readWrite",saved?.groupAccess ?? "readWrite");}}/></ControlHint><Icon size={15} aria-hidden="true"/><CatalogLabel value={item.label}/>
        {saved && <label className="context-access-control" onClick={e => e.stopPropagation()}><span>{readOnly ? "只读" : "读写"}</span><Switch aria-label={`${item.label} 文件组权限`} aria-description="只读时所有子项锁定只读；读写时可逐项限制" checked={!readOnly} disabled={disabled} onChange={e => update(parts,saved.newMemberAccess,e.target.checked ? "readWrite" : "readOnly")}/></label>}
      </summary>
      <ul className="context-catalog-parts">{displayed.map(part => <li key={agentContextKey(part.resource)} className={part.resource.kind === "paperNotes" ? "context-reading-notes" : undefined}>
        <div className="context-catalog-part-row">
          <label><input type="checkbox" aria-label={`使用 ${item.label} · ${part.label}`} checked={part.selected} disabled={disabled} onChange={e => replace(part, {...part,selected:e.target.checked})}/><CatalogLabel value={part.label}/></label>
          <label className="context-access-control"><span>{!readOnly && part.access === "readWrite" ? "读写" : "只读"}</span><Switch aria-label={`${item.label} · ${part.label} 权限`} aria-description={!canWriteAgentContext(part.resource) ? "PDF 始终只读" : readOnly ? "文件组为只读，子项不能开启读写" : "开启读写，关闭只读"} checked={!readOnly && part.access === "readWrite"} disabled={disabled || readOnly || !part.selected || !canWriteAgentContext(part.resource)} onChange={e => replace(part, {...part,access:e.target.checked ? "readWrite" : "readOnly"})}/></label>
        </div>
        {part.resource.kind === "paperNotes" && <details className="context-note-exclusions"><summary><ChevronRight className="context-catalog-chevron" size={12} aria-hidden="true"/>排除项{part.resource.excludedNoteIds.length > 0 ? ` · ${part.resource.excludedNoteIds.length}` : ""}</summary>
          <ul>{notes.map(note => <li key={agentContextKey(note.resource)}><label><input type="checkbox" aria-label={`排除 ${note.label} ${note.resource.resourceId}`} disabled={disabled || !part.selected} checked={part.resource.kind === "paperNotes" && part.resource.excludedNoteIds.includes(note.resource.resourceId)} onChange={e => {
            if (part.resource.kind !== "paperNotes") return;
            replace(part,{...part,resource:{...part.resource,excludedNoteIds:e.target.checked ? [...part.resource.excludedNoteIds,note.resource.resourceId] : part.resource.excludedNoteIds.filter(id => id !== note.resource.resourceId)}});
          }}/><CatalogLabel value={note.label}/></label></li>)}</ul>
        </details>}
      </li>)}</ul>
    </details>
  </li>;
}

function CatalogLabel({value}: {readonly value: string}): React.JSX.Element {
  return <ControlHint content={value}><span className="context-catalog-label">{value}</span></ControlHint>;
}
