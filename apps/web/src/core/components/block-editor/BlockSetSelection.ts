import { Fragment, Slice, type Node } from "@tiptap/pm/model";
import { Selection, SelectionRange, type SelectionBookmark, type Transaction } from "@tiptap/pm/state";
import type { Mappable } from "@tiptap/pm/transform";

interface BlockRange { readonly id: string; readonly from: number; readonly to: number; readonly node: Node }

function selectedRoots(doc: Node, ids: readonly string[]): BlockRange[] {
  const wanted = new Set(ids);
  const roots: BlockRange[] = [];
  doc.descendants((node, from) => {
    if (!node.type.isInGroup("bnBlock") || !wanted.has(String(node.attrs.id))) return true;
    roots.push({ id: String(node.attrs.id), from, to: from + node.nodeSize, node });
    return false; // A selected parent already includes its complete subtree.
  });
  return roots;
}

/** The engine owns block selection, including disjoint roots. Commands consume
 * these ranges through Selection.replace/replaceWith, just like text selection.
 * IDs preserve identity across document edits; no selection enters saved content.
 */
export class BlockSetSelection extends Selection {
  readonly blockIds: readonly string[];
  private readonly roots: readonly BlockRange[];
  override visible = false;

  private constructor(doc: Node, roots: readonly BlockRange[]) {
    const ranges: { from: number; to: number }[] = [];
    for (const root of roots) {
      const previous = ranges.at(-1);
      if (previous?.to === root.from) previous.to = root.to;
      else ranges.push({ from: root.from, to: root.to });
    }
    super(doc.resolve(roots[0]!.from), doc.resolve(roots.at(-1)!.to),
      ranges.map(range => new SelectionRange(doc.resolve(range.from), doc.resolve(range.to))));
    this.roots = roots;
    this.blockIds = roots.map(root => root.id);
  }

  static create(doc: Node, ids: readonly string[], fallback = 0): Selection {
    const roots = selectedRoots(doc, ids);
    return roots.length > 0 ? new BlockSetSelection(doc, roots)
      : Selection.near(doc.resolve(Math.max(0, Math.min(fallback, doc.content.size))));
  }

  override content(): Slice {
    return new Slice(Fragment.fromArray(this.roots.map(root => root.node)), 0, 0);
  }

  /** Finish at the insertion range. Apart from avoiding position drift, this
   * keeps the final changed range adjacent to subsequent native/IME input, so
   * history treats a disjoint replacement as one edit rather than two events.
   */
  private insertionSelection(transaction: Transaction): Selection {
    const first = this.ranges[0]!;
    const ids = this.roots.filter(root => root.from < first.$to.pos).map(root => root.id);
    for (let index = this.ranges.length - 1; index > 0; index--) {
      const range = this.ranges[index]!;
      transaction.deleteRange(range.$from.pos, range.$to.pos);
    }
    return BlockSetSelection.create(transaction.doc, ids, first.$from.pos);
  }

  override replace(transaction: Transaction, content = Slice.empty): void {
    Selection.prototype.replace.call(this.insertionSelection(transaction), transaction, content);
  }

  override replaceWith(transaction: Transaction, node: Node): void {
    Selection.prototype.replaceWith.call(this.insertionSelection(transaction), transaction, node);
  }

  override eq(other: Selection): boolean {
    return other instanceof BlockSetSelection && this.$anchor.doc === other.$anchor.doc
      && this.blockIds.length === other.blockIds.length
      && this.blockIds.every((id, index) => id === other.blockIds[index]);
  }

  override map(doc: Node, mapping: Mappable): Selection {
    return BlockSetSelection.create(doc, this.blockIds, mapping.map(this.anchor));
  }

  override getBookmark(): SelectionBookmark {
    return new BlockSetBookmark(this.blockIds, this.anchor);
  }

  override toJSON(): { type: string; blockIds: readonly string[]; anchor: number } {
    return { type: "researchBlockSet", blockIds: this.blockIds, anchor: this.anchor };
  }

  static override fromJSON(doc: Node, value: { blockIds: string[]; anchor: number }): Selection {
    return BlockSetSelection.create(doc, value.blockIds, value.anchor);
  }
}

class BlockSetBookmark implements SelectionBookmark {
  constructor(private readonly ids: readonly string[], private readonly anchor: number) {}
  map(mapping: Mappable): SelectionBookmark { return new BlockSetBookmark(this.ids, mapping.map(this.anchor)); }
  resolve(doc: Node): Selection { return BlockSetSelection.create(doc, this.ids, this.anchor); }
}

Selection.jsonID("researchBlockSet", BlockSetSelection);
