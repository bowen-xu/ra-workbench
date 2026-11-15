import {createBlockSpec, createExtension} from "@blocknote/core";
import {DOMSerializer, Fragment, Slice, type Node as PmNode} from "@tiptap/pm/model";
import {Plugin} from "@tiptap/pm/state";

import {blockTextEditing} from "./blockTextEditing";
import {blockEditingConstraints} from "./blockEditingConstraints";
import {normalizeRestrictedText, restrictedTextNodeSpec} from "./restrictedBlockText";

/** Fixed title projection of the same HBS document as the body. */
import {DOCUMENT_TITLE} from "./documentTitlePolicy";
export interface DocumentTitleBinding {
  readonly value: string;
}
const titleTextSpec = restrictedTextNodeSpec({tag: "h1", className: "document-title", label: "笔记标题"});
export const documentTitleBlockSpec = createBlockSpec({type: DOCUMENT_TITLE, propSchema: {}, content: "plain"}, {
  meta: {hardBreakShortcut: "none"},
  render: () => {
    const view = DOMSerializer.renderSpec(document, titleTextSpec.toDOM());
    if (!(view.dom instanceof HTMLElement) || !view.contentDOM) throw new Error("restricted_title_view_invalid：标题内容视图无效。");
    return {dom: view.dom, contentDOM: view.contentDOM};
  },
})();
export function documentTitleId(pageId: string): string {return `document-title:${pageId}`;}
export function documentTitleBlock(pageId: string, text: string) {
  return {id: documentTitleId(pageId), type: DOCUMENT_TITLE, props: {}, content: normalizeDocumentTitle(text)};
}
export function documentTitleText(doc: PmNode): string {return doc.firstChild?.firstChild?.firstChild?.textContent ?? "";}
export function normalizeDocumentTitle(text: string): string {
  return normalizeRestrictedText(text, blockEditingConstraints[DOCUMENT_TITLE]);
}

/** One schema policy for every host: fixed first node, plain title text, native
 * selection/history and the legal title → body boundary. No extra editor/undo stack. */
export const documentTextConstraints = blockEditingConstraints;
export const documentTextEditing = () => blockTextEditing(documentTextConstraints);
export function documentTitleExtension(pageId: string) {
  return createExtension(({editor}) => ({key: "document-title", prosemirrorPlugins: [new Plugin({
    props: {
      transformPasted(slice) {
        const convert = (node: PmNode): PmNode => {
          const children: PmNode[] = []; node.forEach(child => children.push(convert(child)));
          if (node.type.name === DOCUMENT_TITLE) return editor.prosemirrorState.schema.nodes.paragraph!.create(null, node.content);
          return node.isLeaf ? node : node.copy(Fragment.from(children));
        };
        const children: PmNode[] = []; slice.content.forEach(node => children.push(convert(node)));
        return new Slice(Fragment.from(children), slice.openStart, slice.openEnd);
      },
    },
    appendTransaction(transactions, previous, state) {
      if (!transactions.some(transaction => transaction.docChanged)) return null;
      const group = state.doc.firstChild;
      if (!group) return null;
      const id = documentTitleId(pageId), tr = state.tr;
      let title: PmNode | undefined, titlePos = 1;
      group.forEach((node, offset) => {if (node.attrs.id === id) {title = node; titlePos = offset + 1;}});
      if (!title) {
        const content = state.schema.nodes[DOCUMENT_TITLE]!.create();
        tr.insert(1, state.schema.nodes.blockContainer!.create({id}, content));
      } else {
        const content = title.firstChild!, text = content.type.name === DOCUMENT_TITLE ? content.textContent : documentTitleText(previous.doc);
        if (content.type.name !== DOCUMENT_TITLE || content.textContent !== text || title.childCount !== 1) {
          const normalized = state.schema.nodes[DOCUMENT_TITLE]!.create(null, text ? state.schema.text(text) : undefined);
          const children: PmNode[] = []; title.forEach((node, _offset, index) => {if (index > 0) node.forEach(child => children.push(child));});
          tr.replaceWith(titlePos, titlePos + title.nodeSize, [state.schema.nodes.blockContainer!.create({id}, normalized), ...children]);
        }
        if (titlePos !== 1) {
          const pos = tr.mapping.map(titlePos, -1), node = tr.doc.nodeAt(pos)!;
          tr.delete(pos, pos + node.nodeSize).insert(1, node);
        }
      }
      return tr.steps.length ? tr : null;
    },
  })]}))();
}
