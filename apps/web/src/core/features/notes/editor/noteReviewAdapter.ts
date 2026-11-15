import { resolveReviewDocument, type ReviewAction, type ReviewDocument } from "@research-assistant/core/document-review";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import type { TextPatchBlockChange } from "../../agent/textPatchGateway";
import { sameNoteProjection } from "./noteReviewProjection";

interface BlockRange { readonly from: number; readonly count: number }
/** Stable-root ranges are provided by the native note diff adapter. Replacement
 * preserves complete blocks and private sidecars instead of importing Markdown. */
export function resolveNoteReviewDocument(snapshot: ReviewDocument<NoteDocumentV1>, action: ReviewAction,
  chunk?: TextPatchBlockChange): ReviewDocument<NoteDocumentV1> {
  return resolveReviewDocument(snapshot, action, {
    equal: sameNoteProjection,
    replace(target, range: BlockRange, source, sourceRange: BlockRange) {
      const blocks = [...target.blocks];
      blocks.splice(range.from, range.count, ...source.blocks.slice(sourceRange.from, sourceRange.from + sourceRange.count));
      return { ...target, blocks };
    },
  }, chunk ? { baseline: { from: chunk.beforeIndex, count: chunk.beforeBlockIds.length },
    current: { from: chunk.afterIndex, count: chunk.afterBlockIds.length } } : undefined);
}
