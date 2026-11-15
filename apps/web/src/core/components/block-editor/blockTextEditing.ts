import {createExtension} from "@blocknote/core";
import {Fragment, type Node} from "@tiptap/pm/model";
import {Plugin, TextSelection, type EditorState, type Transaction} from "@tiptap/pm/state";
import type {EditorView} from "@tiptap/pm/view";

import type {TextBlockConstraint, TextBlockConstraints} from "./blockEditingConstraints";
import {legalRestrictedText, normalizeRestrictedText} from "./restrictedBlockText";
export type BlockTextIntent =
  | {readonly kind: "join"; readonly left: string; readonly right: string}
  | {readonly kind: "split"; readonly block: string; readonly from: number; readonly to: number; readonly next: string; readonly type: string};
export const BLOCK_TEXT_INTENT = "block-text-intent";

interface Location {readonly node: Node; readonly pos: number; readonly parent: Node}
export function textBlockLocation(doc: Node, id: string): Location | undefined {
  let found: Location | undefined;
  doc.descendants((node, pos, parent) => {
    if (node.type.name === "blockContainer" && node.attrs.id === id && parent) {found = {node, pos, parent}; return false;}
    return !found;
  });
  return found;
}
function textContent(content: Node, destination: Node): Fragment | undefined {
  if (destination.type.validContent(content.content)) return content.content;
  // A plain node drops presentation marks, never links or semantic inline atoms.
  const nodes: Node[] = [];
  let compatible = true;
  content.forEach(node => {
    if (!node.isText || node.marks.some(mark => mark.type.name === "link")) compatible = false;
    else nodes.push(node.mark([]));
  });
  const fragment = Fragment.from(nodes);
  return compatible && destination.type.validContent(fragment) ? fragment : undefined;
}
function legal(content: Node, constraint: TextBlockConstraint | undefined): boolean {
  return legalRestrictedText(content.textBetween(0, content.content.size, "\n", "\n"), constraint);
}

/** Replace plain text in-place, including an empty string. The upstream block
 * update API treats an empty string as an omitted update; native transactions
 * must distinguish clearing text from leaving it unchanged. */
export function replaceBlockText(transaction: Transaction, id: string, text: string, constraints: TextBlockConstraints = {}): boolean {
  const block = textBlockLocation(transaction.doc, id), content = block?.node.firstChild;
  if (!block || !content?.isTextblock) return false;
  const fragment = text ? Fragment.from(transaction.doc.type.schema.text(text)) : Fragment.empty;
  const next = content.copy(fragment);
  if (!content.type.validContent(fragment) || !legal(next, constraints[content.type.name])) return false;
  transaction.replaceWith(block.pos + 2, block.pos + 2 + content.content.size, fragment);
  return true;
}

/** One transform for every compatible text boundary. Identity/position rules
 * describe nodes; they do not replace split/join semantics with host keymaps.
 * Also used on a detached accepted document to project the same intent into B. */
export function applyBlockTextIntent(transaction: Transaction, intent: BlockTextIntent, constraints: TextBlockConstraints): boolean {
  const {schema} = transaction.doc.type;
  if (intent.kind === "join") {
    const left = textBlockLocation(transaction.doc, intent.left), right = textBlockLocation(transaction.doc, intent.right);
    if (!left || !right || left.node.childCount !== 1 || constraints[right.node.firstChild!.type.name]?.fixed) return false;
    const $left = transaction.doc.resolve(left.pos);
    const attached = constraints[left.node.firstChild!.type.name]?.splitOutsideParent && $left.depth >= 2
      && $left.node($left.depth - 1).type.name === "blockContainer" && $left.after($left.depth - 1) === right.pos
      && $left.index() === left.parent.childCount - 1;
    if (!attached && (left.parent !== right.parent || left.pos + left.node.nodeSize !== right.pos)) return false;
    const a = left.node.firstChild!, b = right.node.firstChild!;
    if (!a.isTextblock || !b.isTextblock) return false;
    const content = textContent(b, a); if (!content) return false;
    const joined = a.copy(a.content.append(content)); if (!legal(joined, constraints[a.type.name])) return false;
    const children: Node[] = [];
    if (right.node.childCount > 1) right.node.child(1).forEach(child => children.push(child));
    if (attached) {
      transaction.replaceWith(left.pos, left.pos + left.node.nodeSize, left.node.copy(Fragment.from(joined)));
      const at = transaction.mapping.map(right.pos);
      transaction.replaceWith(at, at + right.node.nodeSize, children);
    } else transaction.replaceWith(left.pos, right.pos + right.node.nodeSize, [left.node.copy(Fragment.from(joined)), ...children]);
    transaction.setSelection(TextSelection.create(transaction.doc, left.pos + 2 + a.content.size));
    return true;
  }
  const block = textBlockLocation(transaction.doc, intent.block), type = schema.nodes[intent.type];
  if (!block || !type || block.node.childCount !== 1) return false;
  const content = block.node.firstChild!;
  if (!content.isTextblock || intent.from < 0 || intent.to < intent.from || intent.to > content.content.size) return false;
  const left = content.copy(content.content.cut(0, intent.from)), right = type.create(null, content.content.cut(intent.to));
  if (!type.validContent(right.content) || !legal(left, constraints[content.type.name]) || !legal(right, constraints[type.name])) return false;
  const next = block.node.type.create({id: intent.next}, right);
  if (constraints[content.type.name]?.splitOutsideParent) {
    const $block = transaction.doc.resolve(block.pos);
    if ($block.depth < 2) return false;
    const ownerDepth = $block.depth - 1, owner = $block.node(ownerDepth);
    if (owner.type.name !== "blockContainer") return false;
    const afterOwner = $block.after(ownerDepth);
    transaction.replaceWith(block.pos, block.pos + block.node.nodeSize, block.node.copy(Fragment.from(left)));
    const at = transaction.mapping.map(afterOwner);
    transaction.insert(at, next);
    transaction.setSelection(TextSelection.create(transaction.doc, at + 2));
  } else {
    transaction.replaceWith(block.pos, block.pos + block.node.nodeSize, [block.node.copy(Fragment.from(left)), next]);
    transaction.setSelection(TextSelection.create(transaction.doc, block.pos + left.nodeSize + 4));
  }
  return true;
}

function boundaryIntent(state: EditorState, key: string, constraints: TextBlockConstraints): BlockTextIntent | false | undefined {
  const {selection} = state, {$from, $to} = selection;
  if (!$from.parent.isTextblock || $from.parent !== $to.parent || $from.depth < 2) return undefined;
  const content = $from.parent, block = $from.node($from.depth - 1);
  if (block.type.name !== "blockContainer") return undefined;
  const rule = constraints[content.type.name];
  // Lists, headings, tables and code retain their schema's specialized commands.
  // Paragraphs and declared constrained text nodes use this shared engine.
  if (key === "Enter") {
    if (content.type.name !== "paragraph" && !rule?.splitType || block.childCount !== 1) return undefined;
    return {kind: "split", block: block.attrs.id, from: $from.parentOffset, to: $to.parentOffset,
      next: crypto.randomUUID(), type: rule?.splitType ?? "paragraph"};
  }
  if (!selection.empty || key !== "Backspace" && key !== "Delete") return undefined;
  const backward = key === "Backspace";
  if ($from.parentOffset !== (backward ? 0 : content.content.size)) return undefined;
  if (content.type.name !== "paragraph" && !rule?.fixed) return undefined;
  const group = $from.node($from.depth - 2), index = $from.index($from.depth - 2);
  const neighbor = index + (backward ? -1 : 1);
  if (neighbor < 0 || neighbor >= group.childCount) {
    if (!backward && rule?.splitOutsideParent && $from.depth >= 4) {
      const ownerDepth = $from.depth - 3, outer = $from.node(ownerDepth - 1), index = $from.index(ownerDepth - 1);
      const next = index + 1 < outer.childCount ? outer.child(index + 1) : undefined;
      if (next?.firstChild?.isTextblock) return {kind: "join", left: block.attrs.id, right: next.attrs.id};
    }
    return rule?.fixed ? false : undefined;
  }
  const other = group.child(neighbor);
  const attached = backward && other.childCount > 1 ? other.child(1).lastChild : undefined;
  const prior = attached?.firstChild && constraints[attached.firstChild.type.name]?.splitOutsideParent ? attached : other;
  const left = backward ? prior : block, right = backward ? block : other;
  if (left.childCount !== 1 || !left.firstChild?.isTextblock || !right.firstChild?.isTextblock) return undefined;
  if (backward && $from.depth > 3) {
    const owner = $from.node($from.depth - 3).firstChild;
    const toggle = owner?.type.name === "toggleListItem" || owner?.type.name === "heading" && owner.attrs.isToggleable === true;
    // A nonempty, non-final toggle child joins the previous text sibling.
    // Reuse this intent (including HBS baseline projection), rather than letting
    // BlockNote lift it. The final row always lifts, empty or not; the note
    // surface handles that position before this shared merge command.
    if (!toggle || content.content.size === 0 || index === group.childCount - 1) return undefined;
  }
  const fixedBoundary = constraints[left.firstChild.type.name]?.fixed || constraints[right.firstChild.type.name]?.fixed;
  if (!fixedBoundary && right.childCount !== 1) return undefined;
  return {kind: "join", left: left.attrs.id, right: right.attrs.id};
}

export function blockTextEditing(constraints: TextBlockConstraints = {}) {
  const paste = (view: EditorView, event: ClipboardEvent): boolean => {
    const {$from, $to} = view.state.selection, rule = constraints[$from.parent.type.name];
    if (!rule?.plainText || $from.parent !== $to.parent) return false;
    const text = event.clipboardData?.getData("text/plain");
    if (view.editable && text) view.dispatch(view.state.tr.insertText(normalizeRestrictedText(text.replace(/\r\n?/gu, "\n"), rule)).scrollIntoView());
    return true;
  };
  const run = (view: EditorView, key: string) => {
    const intent = boundaryIntent(view.state, key, constraints);
    if (intent === undefined) return false;
    if (intent === false) return true;
    const tr = view.state.tr;
    if (applyBlockTextIntent(tr, intent, constraints)) view.dispatch(tr.setMeta(BLOCK_TEXT_INTENT, intent).scrollIntoView());
    return true;
  };
  return createExtension(({editor}) => {
    let compositionEndedAt: number | undefined;
    return {key: "block-text-editing",
    mount({dom}) {
      // BlockNote owns a DOM-level HTML/Markdown paste handler. Restricted
      // text fields share this admission before that richer body parser.
      const capture = (event: ClipboardEvent) => {
        if (!paste(editor.prosemirrorView, event)) return;
        event.preventDefault(); event.stopPropagation();
      };
      dom.addEventListener("paste", capture, true);
      return () => dom.removeEventListener("paste", capture, true);
    }, prosemirrorPlugins: [new Plugin({
    filterTransaction(transaction) {
      if (!transaction.docChanged) return true;
      let valid = true;
      transaction.doc.descendants(node => {if (constraints[node.type.name] && !legal(node, constraints[node.type.name])) valid = false;});
      return valid;
    }, props: {
    handleKeyDown(view, event) {
      if (!view.editable || view.composing || event.isComposing || event.keyCode === 229) return false;
      // Reaching this handler means the engine admitted an ordinary keydown.
      // A deliberate next Enter must not be mistaken for IME confirmation.
      compositionEndedAt = undefined;
      if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return false;
      // Incompatible content/limits cannot fall through to a lossy native join.
      return run(view, event.key);
    },
    handlePaste: paste,
    handleDOMEvents: {
      compositionstart() {compositionEndedAt = undefined; return false;},
      compositionend() {compositionEndedAt = Date.now(); return false;},
      keydown(view, event) {
        if (event.key === "Enter" && constraints[view.state.selection.$from.parent.type.name]?.splitType
          && (view.composing || event.isComposing || event.keyCode === 229)) {
          event.stopImmediatePropagation(); return true;
        }
        return false;
      },
      beforeinput(view, raw) {
        const event = raw as InputEvent;
        if (!view.editable || view.composing || event.isComposing) return false;
        // ProseMirror guards Safari keydown for 500ms after compositionend
        // (prosemirror-view/src/input.ts, inOrNearComposition). beforeinput
        // bypasses that admission, so consume the matching terminal paragraph
        // event once, leaving the committed text and DOM input to the engine.
        if (event.inputType === "insertParagraph" && compositionEndedAt !== undefined && Date.now() - compositionEndedAt < 500) {
          compositionEndedAt = undefined;
          event.preventDefault(); return true;
        }
        const key = ({insertParagraph: "Enter", deleteContentBackward: "Backspace", deleteContentForward: "Delete"} as Record<string, string>)[event.inputType];
        if (key && run(view, key)) {event.preventDefault(); return true;} return false;
      },
    },
  }})]};
  })();
}
