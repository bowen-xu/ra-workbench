import { ControlHint } from "../../../components/ControlHint";
import { useState } from "react";
import { ChevronDown, ChevronRight, FileText, GripVertical, Star } from "lucide-react";
import { notePageId, type NotePage, type NotePageId } from "@research-assistant/core/notes";

interface NotePageTreeProps {
  readonly pages: readonly NotePage[];
  readonly selectedId: NotePageId | undefined;
  readonly onSelect: (id: NotePageId) => void;
  readonly onPin?: ((id: NotePageId) => void) | undefined;
  readonly onMove: (id: NotePageId, parentId: NotePageId | undefined, position: number) => void;
}

export type PageDropZone = "before" | "inside" | "after";

export interface PageDropDestination {
  readonly parentId: NotePageId | undefined;
  readonly position: number;
}

const DRAG_TYPE = "application/x-research-assistant-note-page-id";

export function pageTreeKeyboardTarget(current: number, count: number, key: string): number | undefined {
  if (count <= 0 || current < 0 || current >= count) return undefined;
  if (key === "ArrowDown") return Math.min(current + 1, count - 1);
  if (key === "ArrowUp") return Math.max(current - 1, 0);
  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  return undefined;
}

export function pageDropZone(pointerY: number, top: number, height: number): PageDropZone {
  if (height <= 0) return "inside";
  const ratio = (pointerY - top) / height;
  if (ratio < 0.25) return "before";
  if (ratio > 0.75) return "after";
  return "inside";
}

function isDescendant(pages: readonly NotePage[], candidateId: NotePageId, ancestorId: NotePageId): boolean {
  let current = pages.find((page) => page.id === candidateId);
  const visited = new Set<NotePageId>();
  while (current?.parentPageId !== undefined && !visited.has(current.parentPageId)) {
    if (current.parentPageId === ancestorId) return true;
    visited.add(current.parentPageId);
    current = pages.find((page) => page.id === current?.parentPageId);
  }
  return false;
}

export function pageDropDestination(
  pages: readonly NotePage[],
  draggedId: NotePageId,
  targetId: NotePageId,
  zone: PageDropZone,
): PageDropDestination | undefined {
  const dragged = pages.find((page) => page.id === draggedId);
  const target = pages.find((page) => page.id === targetId);
  if (dragged === undefined || target === undefined || dragged.id === target.id) return undefined;
  if (isDescendant(pages, target.id, dragged.id)) return undefined;

  if (zone === "inside") {
    const childCount = pages.filter((page) => page.parentPageId === target.id && page.id !== dragged.id).length;
    return { parentId: target.id, position: childCount };
  }

  let position = target.position + (zone === "after" ? 1 : 0);
  if (dragged.parentPageId === target.parentPageId && dragged.position < position) position -= 1;
  return { parentId: target.parentPageId, position };
}

interface PageTreeItemsProps extends NotePageTreeProps {
  readonly parentId?: NotePageId;
  readonly collapsedIds: ReadonlySet<NotePageId>;
  readonly dropTarget: { readonly id: NotePageId; readonly zone: PageDropZone } | undefined;
  readonly onToggleCollapsed: (id: NotePageId) => void;
  readonly onDropTargetChange: (target: { readonly id: NotePageId; readonly zone: PageDropZone } | undefined) => void;
}

function PageTreeItems(props: PageTreeItemsProps): React.JSX.Element {
  const children = props.pages
    .filter((page) => page.parentPageId === props.parentId)
    .sort((left, right) => left.position - right.position);
  return (
    <ul className={props.parentId === undefined ? "note-page-tree" : "note-page-children"} role={props.parentId === undefined ? "tree" : "group"}>
      {children.map((page) => {
        const hasChildren = props.pages.some((candidate) => candidate.parentPageId === page.id);
        const isCollapsed = props.collapsedIds.has(page.id);
        const zone = props.dropTarget?.id === page.id ? props.dropTarget.zone : undefined;
        return <li
          aria-selected={page.id === props.selectedId}
          className={zone === undefined ? undefined : `drop-${zone}`}
          data-drop-zone={zone}
          key={page.id}
          onDragLeave={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget as Node | null)) props.onDropTargetChange(undefined);
          }}
          onDragOver={(event) => {
            if (!event.dataTransfer.types.includes(DRAG_TYPE)) return;
            event.preventDefault();
            event.stopPropagation();
            const rect = event.currentTarget.querySelector<HTMLElement>(":scope > .note-page-row")?.getBoundingClientRect();
            if (rect === undefined) return;
            const nextZone = pageDropZone(event.clientY, rect.top, rect.height);
            event.dataTransfer.dropEffect = "move";
            props.onDropTargetChange({ id: page.id, zone: nextZone });
          }}
          onDrop={(event) => {
            event.preventDefault();
            event.stopPropagation();
            const value = event.dataTransfer.getData(DRAG_TYPE);
            const activeZone = props.dropTarget?.id === page.id ? props.dropTarget.zone : "inside";
            if (value !== "") {
              const draggedId = notePageId(value);
              const destination = pageDropDestination(props.pages, draggedId, page.id, activeZone);
              if (destination !== undefined) props.onMove(draggedId, destination.parentId, destination.position);
            }
            props.onDropTargetChange(undefined);
          }}
          role="treeitem"
        >
          <div className="note-page-row">
            <ControlHint content="拖动页面"><button
              aria-label={`拖动“${page.title}”`}
              className="note-page-drag-handle"
              draggable
              onDragEnd={() => {
                props.onDropTargetChange(undefined);
              }}
              onDragStart={(event) => {
                event.dataTransfer.effectAllowed = "move";
                event.dataTransfer.setData(DRAG_TYPE, page.id);
              }}
              type="button"
            ><GripVertical aria-hidden="true" size={15} /></button></ControlHint>
            {hasChildren ? <button
              aria-expanded={!isCollapsed}
              aria-label={`${isCollapsed ? "展开" : "折叠"}${page.title}`}
              className="note-page-disclosure"
              onClick={() => props.onToggleCollapsed(page.id)}
              type="button"
            >{isCollapsed ? <ChevronRight aria-hidden="true" size={14} /> : <ChevronDown aria-hidden="true" size={14} />}</button> : <span className="note-page-disclosure-spacer" />}
            <button
              aria-current={page.id === props.selectedId ? "page" : undefined}
              className={page.id === props.selectedId ? "selected" : ""}
              data-note-page-id={page.id}
              onClick={() => props.onSelect(page.id)}
              onDoubleClick={() => props.onPin?.(page.id)}
              onKeyDown={(event) => {
                if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
                const tree = event.currentTarget.closest("[role='tree']");
                const buttons = tree === null ? [] : [...tree.querySelectorAll<HTMLButtonElement>("[data-note-page-id]")];
                const target = pageTreeKeyboardTarget(buttons.indexOf(event.currentTarget), buttons.length, event.key);
                if (target === undefined) return;
                event.preventDefault();
                buttons[target]?.focus();
              }}
              type="button"
            >
              {page.isFavorite ? <Star aria-hidden="true" fill="currentColor" size={14} /> : <FileText aria-hidden="true" size={14} />}
              <span>{page.title}</span>
            </button>
          </div>
          {!isCollapsed && <PageTreeItems {...props} parentId={page.id} />}
        </li>;
      })}
    </ul>
  );
}

export function NotePageTree(props: NotePageTreeProps): React.JSX.Element {
  const [collapsedIds, setCollapsedIds] = useState<ReadonlySet<NotePageId>>(new Set());
  const [dropTarget, setDropTarget] = useState<{ readonly id: NotePageId; readonly zone: PageDropZone }>();
  return <PageTreeItems
    {...props}
    collapsedIds={collapsedIds}
    dropTarget={dropTarget}
    onDropTargetChange={setDropTarget}
    onToggleCollapsed={(id) => setCollapsedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    })}
  />;
}
