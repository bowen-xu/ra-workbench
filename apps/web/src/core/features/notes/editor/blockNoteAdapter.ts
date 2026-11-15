import { blocksToEditor, decodeNoteDocument, editorToBlocks, type NoteDocumentV1 } from "@research-assistant/core/notes";
import type { NoteEditorBlock, PartialNoteEditorBlock } from "./noteEditorSchema";
import {documentBodyBlocks} from "../../../components/block-editor/documentTitlePolicy";
import {isDocumentTitle} from "../../../components/block-editor/documentTitlePolicy";
import {withCaptionBlocks, withoutCaptionBlocks} from "../../../components/block-editor/captionBlockProjection";

/** The pinned runtime emits unstyled text items for plain blocks although its
 * public type declares string. Validate the actual boundary without flattening
 * semantic nodes or silently dropping malformed title content. */
function plainTitleText(value: unknown): string {
  if (typeof value === "string") return value;
  const invalid = () => new Error("note_title_projection_invalid：标题文本投影无效。");
  if (!Array.isArray(value)) throw invalid();
  return value.map((part: unknown) => {
    if (!part || typeof part !== "object" || Array.isArray(part)) throw invalid();
    const item = part as Record<string, unknown>, styles = item.styles;
    if (item.type !== "text" || typeof item.text !== "string" || !styles || typeof styles !== "object"
      || Array.isArray(styles) || Object.keys(styles).length > 0
      || Object.keys(item).some(key => !["type", "text", "styles"].includes(key))) throw invalid();
    return item.text;
  }).join("");
}

export function noteDocumentToBlockNote(document: NoteDocumentV1,
  schema?: Readonly<Record<string, { readonly propSchema: Readonly<Record<string, unknown>> }>>,
): PartialNoteEditorBlock[] {
  const project = (blocks: ReturnType<typeof blocksToEditor>): ReturnType<typeof blocksToEditor> => blocks.map(block => ({
    ...block,
    // The native editor normalizes partial blocks, but its HTML serializer
    // passes props directly to React renderers. Only expose declared props at
    // that boundary; hidden domain properties remain in the trusted document.
    ...(schema ? { props: Object.fromEntries(Object.entries(block.props ?? {})
      .filter(([key]) => Object.hasOwn(schema[block.type]?.propSchema ?? {}, key))) } : {}),
    ...(block.children ? { children: project([...block.children]) } : {}),
  }));
  return project(withCaptionBlocks(blocksToEditor(document.blocks))) as PartialNoteEditorBlock[];
}
export function blockNoteToNoteDocument(
  pageId: NoteDocumentV1["pageId"],
  blocks: readonly NoteEditorBlock[],
  options: { readonly validChildPages?: readonly { readonly blockId: string; readonly targetId: string }[] } = {},
): NoteDocumentV1 {
  const title = blocks.find(isDocumentTitle);
  const text = title === undefined ? undefined : plainTitleText(title.content);
  return decodeNoteDocument({ schemaVersion: 1, pageId, ...(text === undefined ? {} : {title: text}), blocks: editorToBlocks(withoutCaptionBlocks(documentBodyBlocks(blocks)), options) });
}

/** The production codec describes rendered schema props. Retain non-rendered
 * domain props by stable block ID when a controlled host reads the view. */
export function retainNoteProperties(value: NoteDocumentV1, trusted: NoteDocumentV1,
  schema?: Readonly<Record<string, { readonly propSchema: Readonly<Record<string, { readonly default: unknown } | undefined>> }>>,
): NoteDocumentV1 {
  const originals = new Map<string, NoteDocumentV1["blocks"][number]>();
  const collect = (blocks: NoteDocumentV1["blocks"]) => blocks.forEach(block => { originals.set(block.id,block); collect(block.children); });
  collect(trusted.blocks);
  const walk = (blocks: NoteDocumentV1["blocks"]): NoteDocumentV1["blocks"] => blocks.map(block => {
    const original = originals.get(block.id), prior = original?.type === block.type ? original.props : undefined;
    const props = { ...prior, ...block.props };
    // BlockNote materializes omitted schema defaults in every native document.
    // Keep the trusted sparse representation for unchanged properties while
    // retaining explicit edits, including an explicit reset of a stored prop.
    if (prior) for (const [key, rule] of Object.entries(schema?.[block.type]?.propSchema ?? {})) {
      if (rule && !(key in prior) && props[key] === rule.default) delete props[key];
    }
    return { ...block, props, children: walk(block.children) };
  });
  return { ...value, ...(value.title === undefined && trusted.title !== undefined ? {title: trusted.title} : {}), blocks: walk(value.blocks) };
}
