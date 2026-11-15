import {blockToNode} from "@blocknote/core";
import {EditorState} from "@tiptap/pm/state";
import type {Node} from "@tiptap/pm/model";
import {mapTextPosition} from "@research-assistant/core/hbs";
import type {NoteDocumentV1} from "@research-assistant/core/notes";
import {applyBlockTextIntent, textBlockLocation, type BlockTextIntent} from "../../../components/block-editor/blockTextEditing";
import {documentTitleBlock, documentTextConstraints} from "../../../components/block-editor/documentTitle";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {noteDocumentToBlockNote} from "./blockNoteAdapter";
import {noteReviewDocumentFromState} from "./noteReviewProjection";

/** Replay the edit command, not its resulting text, against accepted B.
 * No live editor, history or second writer is created. Pending text therefore
 * remains pending when a split/join moves it across a block boundary. */
export function projectNoteBlockTextIntent(before: Node, baseline: NoteDocumentV1, intent: BlockTextIntent): NoteDocumentV1 | undefined {
  const hasTitle = before.firstChild?.firstChild?.firstChild?.type.name === "documentTitle";
  const blocks = [...(hasTitle ? [documentTitleBlock(baseline.pageId, baseline.title ?? "无标题")] : []), ...noteDocumentToBlockNote(baseline)];
  if (!blocks.length) return baseline;
  const schema = before.type.schema;
  const doc = before.type.create(null, schema.nodes.blockGroup!.create(null, blocks.map(block => blockToNode(block, schema))));
  let projected = intent;
  if (intent.kind === "split") {
    const original = textBlockLocation(before, intent.block), accepted = textBlockLocation(doc, intent.block);
    if (!accepted) return baseline;
    if (!original) return undefined;
    const text = (node: Node) => node.firstChild!.textBetween(0, node.firstChild!.content.size, "\n", "\uFFFC");
    const from = mapTextPosition(text(original.node), text(accepted.node), intent.from, -1, directTextDiff);
    const to = intent.to === intent.from ? from : mapTextPosition(text(original.node), text(accepted.node), intent.to, 1, directTextDiff);
    projected = {...intent, from, to};
  } else if (!textBlockLocation(doc, intent.right)) return baseline;
  const transaction = EditorState.create({doc, schema}).tr;
  if (!applyBlockTextIntent(transaction, projected, documentTextConstraints)) return undefined;
  return noteReviewDocumentFromState(baseline.pageId, transaction.doc, baseline);
}
