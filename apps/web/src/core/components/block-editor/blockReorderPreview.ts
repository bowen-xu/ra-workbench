import {blockNoteDragDom, type BlockDragDom} from "./blockDragDom";
import { reorderRowOffsets, type ReorderRowGeometry } from "../reorderLayout";
import type { BlockPlacement } from "./moveBlocks";

interface RowTree { readonly id: string; readonly children: readonly RowTree[] }
interface BlockGeometry extends ReorderRowGeometry {
  readonly parentId: string | undefined;
  readonly prefix: number;
  readonly suffix: number;
}

/** Preview the same tree move without changing ProseMirror or creating an undo step. */
function previewBlockReorder(tree: readonly RowTree[], sourceIds: readonly string[], target: BlockPlacement): readonly RowTree[] | undefined {
  const sources = new Set(sourceIds), moved: RowTree[] = [];
  function remove(group: readonly RowTree[]): RowTree[] {
    return group.flatMap(node => {
      if (sources.has(node.id)) { moved.push(node); return []; }
      return [{ ...node, children: remove(node.children) }];
    });
  }
  const remaining = remove(tree);
  if (moved.length !== sources.size) return undefined;
  let inserted = false;
  function insert(group: readonly RowTree[]): RowTree[] {
    return group.flatMap(node => {
      if (node.id === target.blockId) {
        inserted = true;
        if (target.placement === "inside") return [{ ...node, children: [...node.children, ...moved] }];
        return target.placement === "before" ? [...moved, node] : [node, ...moved];
      }
      return [{ ...node, children: insert(node.children) }];
    });
  }
  const result = insert(remaining);
  return inserted ? result : undefined;
}

/** Uses the queue's displacement calculation and motion tokens. Styling lives
 * outside the editor DOM so its mutation observer cannot redraw saved blocks. */
export function createBlockReorderPreview(editor: HTMLElement, sourceIds: readonly string[], dom: BlockDragDom = blockNoteDragDom): {
  update(target: BlockPlacement | undefined): { readonly shift: number; readonly growth: number };
  dispose(): void;
} {
  const surface = dom.scope(editor);
  const instance = surface?.instance;
  const elements = [...editor.querySelectorAll<HTMLElement>(dom.outerSelector)]
    .filter(element => !dom.fixed(element.querySelector<HTMLElement>(dom.contentSelector) ?? element));
  const idOf = dom.idOf;
  const rowElements = new Map(elements.flatMap(element => { const id = idOf(element); return id ? [[id, element] as const] : []; }));
  const groups = new Map<string | undefined, string[]>();
  const geometry = new Map<string, BlockGeometry>();
  for (const [id, element] of rowElements) {
    const parent = element.parentElement?.closest<HTMLElement>(dom.outerSelector);
    const parentId = parent && editor.contains(parent) ? idOf(parent) : undefined;
    const group = groups.get(parentId) ?? []; group.push(id); groups.set(parentId, group);
    const rect = element.getBoundingClientRect();
    geometry.set(id, { id, top: rect.top, height: rect.height, parentId, prefix: rect.height, suffix: 0 });
  }
  const gapFor = (parentId: string | undefined): number => {
    const ids = groups.get(parentId) ?? [];
    const first = geometry.get(ids[0] ?? ""), second = geometry.get(ids[1] ?? "");
    return first && second ? Math.max(0, second.top - first.top - first.height) : 4;
  };
  for (const [parentId, children] of groups) {
    if (!parentId) continue;
    const parent = geometry.get(parentId), first = geometry.get(children[0] ?? ""), last = geometry.get(children.at(-1) ?? "");
    if (parent && first && last) geometry.set(parentId, { ...parent, prefix: first.top - parent.top, suffix: parent.top + parent.height - last.top - last.height });
  }
  const treeFor = (parentId?: string): RowTree[] => (groups.get(parentId) ?? []).map(id => ({ id, children: treeFor(id) }));
  const tree = treeFor();
  const style = editor.ownerDocument.createElement("style");
  style.dataset.blockReorderPreview = instance ?? "";
  const scope = surface?.selector ?? "";
  const selector = (id: string): string => `${scope} ${dom.rowSelector(id)}`;
  const dim = sourceIds.map(id => `${selector(id)} { opacity: var(--reorder-drag-opacity); border-radius:6px; background:#eaf2ed; }`).join("\n");
  let previous: string | undefined;
  let feedback = { shift: 0, growth: 0 };
  return {
    update(target) {
      if (instance === undefined) return feedback;
      const key = target ? `${target.blockId}:${target.placement}` : "";
      if (previous === key) return feedback;
      previous = key;
      const planned = target ? previewBlockReorder(tree, sourceIds, target) : undefined;
      const heights = new Map<string, number>(), absolute = new Map<string, number>();
      function height(node: RowTree): number {
        const row = geometry.get(node.id);
        if (!row) return 0;
        const value = row.prefix + row.suffix + node.children.reduce((sum, child) => sum + height(child), 0)
          + Math.max(0, node.children.length - 1) * gapFor(node.id);
        heights.set(node.id, value); return value;
      }
      function layout(group: readonly RowTree[], top: number, parentId?: string): void {
        const rows = group.flatMap(node => { const row = geometry.get(node.id); return row ? [{ ...row, height: heights.get(node.id) ?? row.height }] : []; });
        const offsets = reorderRowOffsets(rows, group.map(node => node.id), { top, gap: gapFor(parentId) });
        for (const node of group) {
          const row = geometry.get(node.id); if (!row) continue;
          const offset = offsets.get(node.id) ?? 0;
          absolute.set(node.id, offset);
          layout(node.children, row.top + offset + row.prefix, node.id);
        }
      }
      if (planned) { planned.forEach(height); layout(planned, geometry.get(tree[0]?.id ?? "")?.top ?? 0); }
      feedback = { shift: planned && target ? absolute.get(target.blockId) ?? 0 : 0,
        growth: planned && target ? (heights.get(target.blockId) ?? 0) - (geometry.get(target.blockId)?.height ?? 0) : 0 };
      const rules = [...absolute].map(([id, offset]) => {
        const parent = geometry.get(id)?.parentId;
        const local = offset - (parent ? absolute.get(parent) ?? 0 : 0);
        return `${selector(id)} { transform: translateY(${local}px); }`;
      });
      style.textContent = `${scope} ${dom.outerSelector} { transition: transform var(--reorder-motion-duration) var(--reorder-motion-easing), opacity var(--reorder-motion-duration) ease; }\n${dim}\n${rules.join("\n")}`;
      if (!style.isConnected) editor.ownerDocument.head.append(style);
      return feedback;
    },
    dispose() { style.remove(); },
  };
}
