import {addNodeAndExtensionsToSpec, createBlockSpec, createExtension} from "@blocknote/core";
import {Fragment, Slice, type Node} from "@tiptap/pm/model";
import {Plugin, TextSelection, type Transaction} from "@tiptap/pm/state";
import {Decoration, DecorationSet} from "@tiptap/pm/view";
import {BLOCK_CAPTION, captionBlockId, hasBlockCaption} from "./captionBlockProjection";
import {blockTextEditing} from "./blockTextEditing";
import {blockEditingConstraints} from "./blockEditingConstraints";

export const captionNodeText = (node: Node): string => node.textBetween(0, node.content.size, "\n", "\n");

/** The caption is a native text block inside its resource's blockGroup. Only
 * field projection is special; caret, DOM input, selection and history aren't. */
const definition = createBlockSpec({type: BLOCK_CAPTION, propSchema: {ownerId: {default: ""}}, content: "inline"}, {
  render: (block, editor) => {
    const dom = document.createElement("div");
    dom.className = "block-caption block-caption-input";
    dom.dataset.restrictedBlockText = "true";
    dom.dataset.placeholder = "Write a caption…";
    dom.setAttribute("role", "textbox");
    dom.setAttribute("aria-label", String(editor.getBlock(block.props.ownerId)?.type) === "table" ? "表格说明" : "图片说明");
    dom.setAttribute("aria-multiline", "true");
    return {dom, contentDOM: dom, ignoreMutation: mutation => mutation.type === "attributes"};
  },
})();
const spec = addNodeAndExtensionsToSpec(definition.config, definition.implementation);

function captionContainer(owner: Node, schema: Node["type"]["schema"]): Node {
  const text = String(owner.firstChild!.attrs.caption ?? "");
  const content = schema.nodes[BLOCK_CAPTION]!.create({ownerId: owner.attrs.id}, text ? schema.text(text) : undefined);
  return schema.nodes.blockContainer!.create({id: captionBlockId(owner.attrs.id)}, content);
}

/** Owner attributes are the portable boundary, not another editing session.
 * Native edits and external attribute installations reconcile in the same
 * transaction/history group; projection never writes to storage. */
export function projectCaptionFields(tr: Transaction, previous?: Node, revealOwner?: string): Transaction {
  const prior = new Map<string, Node>();
  previous?.descendants(node => {if (node.type.name === "blockContainer") prior.set(node.attrs.id, node);});
  const owners: {node: Node; pos: number}[] = [];
  tr.doc.descendants((node, pos) => {
    if (node.type.name === "blockContainer" && hasBlockCaption(node.firstChild?.type.name ?? "")) owners.push({node, pos});
  });
  for (const {node: owner, pos} of owners.reverse()) {
    const content = owner.firstChild!, old = prior.get(owner.attrs.id)?.firstChild;
    const group = owner.childCount > 1 ? owner.child(1) : undefined;
    const caption = group?.firstChild;
    const expectedId = captionBlockId(owner.attrs.id);
    const at = pos + 1 + content.nodeSize;
    const attached = caption?.attrs.id === expectedId && caption.firstChild?.type.name === BLOCK_CAPTION;
    const active = attached && tr.selection.from >= at + 1 && tr.selection.to <= at + 1 + caption.nodeSize;
    const value = String(content.attrs.caption ?? "");
    const external = old && old.attrs.caption !== content.attrs.caption;
    const text = attached ? captionNodeText(caption.firstChild!) : value;
    // An absent field must also be absent from native navigation and boundary
    // commands. Keep an empty node only while explicitly editing that field.
    if (!(external ? value : text) && !active && revealOwner !== owner.attrs.id) {
      if (value) tr.setNodeMarkup(pos + 1, undefined, {...content.attrs, caption: ""});
      if (attached) tr.delete(group!.childCount === 1 ? at : at + 1,
        group!.childCount === 1 ? at + group!.nodeSize : at + 1 + caption.nodeSize);
      continue;
    }
    if (caption?.attrs.id !== expectedId || caption.firstChild?.type.name !== BLOCK_CAPTION) {
      const attached = captionContainer(owner, tr.doc.type.schema);
      if (group) tr.insert(at + 1, attached);
      else tr.insert(at, tr.doc.type.schema.nodes.blockGroup!.create(null, attached));
      continue;
    }
    if (text === value) continue;
    if (external) {
      // An external field update owns the projection, including clearing it.
      const at = pos + content.nodeSize + 4;
      tr.replaceWith(at, at + caption.firstChild.content.size, value ? tr.doc.type.schema.text(value) : Fragment.empty);
    } else tr.setNodeMarkup(pos + 1, undefined, {...content.attrs, caption: text});
  }
  return tr;
}

const captionProjection = createExtension(({editor}) => ({key: "caption-block-projection",
  mount: () => {
    const tr = projectCaptionFields(editor.prosemirrorState.tr);
    if (tr.steps.length) editor.prosemirrorView.dispatch(tr.setMeta("addToHistory", false));
  },
  prosemirrorPlugins: [new Plugin({
    appendTransaction(transactions, before, after) {
      if (!transactions.some(tr => tr.docChanged || tr.selectionSet)) return null;
      const tr = projectCaptionFields(after.tr, before.doc);
      if (!transactions.some(tr => tr.docChanged)) tr.setMeta("addToHistory", false);
      return tr.steps.length ? tr : null;
    },
    filterTransaction(tr) {
      if (!tr.docChanged) return true;
      let legal = true;
      tr.doc.descendants((node, pos, parent) => {
        if (node.type.name === "blockContainer" && String(node.attrs.id).startsWith("block-caption:")
          && node.firstChild?.type.name !== BLOCK_CAPTION) legal = false;
        if (node.type.name !== BLOCK_CAPTION) return;
        const $pos = tr.doc.resolve(pos), owner = $pos.depth >= 3 ? $pos.node($pos.depth - 2) : undefined;
        if (captionNodeText(node).length > 4000 || parent?.childCount !== 1 || parent.attrs.id !== captionBlockId(node.attrs.ownerId)
          || owner?.attrs.id !== node.attrs.ownerId || $pos.index($pos.depth - 1) !== 0
          || !hasBlockCaption(owner?.firstChild?.type.name ?? "")) legal = false;
      });
      return legal;
    },
    props: {
      transformPasted(slice) {
        const project = (node: Node, attached = false): Node | undefined => {
          if (node.type.name === "blockContainer" && node.firstChild?.type.name === BLOCK_CAPTION) {
            if (attached) return undefined;
            return node.type.create(null, editor.prosemirrorState.schema.nodes.paragraph!.create(null, node.firstChild.content));
          }
          if (node.isLeaf) return node;
          const children: Node[] = [];
          const media = node.type.name === "blockContainer" && hasBlockCaption(node.firstChild?.type.name ?? "");
          node.forEach(child => {const next = project(child, attached || media); if (next) children.push(next);});
          if (node.type.name === "blockGroup" && !children.length) return undefined;
          return node.copy(Fragment.from(children));
        };
        const children: Node[] = [];
        slice.content.forEach(child => {const next = project(child); if (next) children.push(next);});
        return new Slice(Fragment.from(children), slice.openStart, slice.openEnd);
      },
      decorations(state) {
      const decorations: Decoration[] = [];
      state.doc.descendants((node, pos) => {
        if (node.type.name !== "blockContainer" || node.firstChild?.type.name !== BLOCK_CAPTION) return;
        const active = state.selection.from >= pos && state.selection.to <= pos + node.nodeSize;
        decorations.push(Decoration.node(pos, pos + node.nodeSize, {class: `block-caption-container${!node.firstChild.content.size && (!active || !editor.isEditable) ? " block-caption-hidden" : ""}`}));
      });
      return DecorationSet.create(state.doc, decorations);
    }},
  })],
}));

export const captionBlockSpec = {...spec, extensions: [...(spec.extensions ?? []), blockTextEditing(blockEditingConstraints), captionProjection()],
  implementation: {...spec.implementation, node: spec.implementation.node.extend({content: "(text|hardBreak)*", marks: ""})}};

export function focusBlockCaption(editor: {readonly isEditable: boolean; readonly prosemirrorView: import("@tiptap/pm/view").EditorView}, owner: string): void {
  if (!editor.isEditable) return;
  const view = editor.prosemirrorView;
  const tr = projectCaptionFields(view.state.tr, undefined, owner);
  let position: number | undefined;
  tr.doc.descendants((node, pos) => {if (node.type.name === "blockContainer" && node.attrs.id === captionBlockId(owner)) {position = pos + 2; return false;}});
  if (position === undefined) return;
  view.dispatch(tr.setSelection(TextSelection.create(tr.doc, position)).setMeta("addToHistory", false).scrollIntoView());
  view.focus();
}
