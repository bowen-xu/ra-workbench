import type {PortableEditorBlock} from "@research-assistant/core/notes";

export const BLOCK_CAPTION = "blockCaption";
export const captionBlockId = (owner: string): string => `block-caption:${owner}`;
export const hasBlockCaption = (type: string): boolean => type === "managedImage" || type === "table";

/** Runtime text blocks project an existing resource field, like documentTitle.
 * They never become independent persisted paragraphs or resource identities. */
export function captionBlockText(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) throw new Error("note_caption_projection_invalid：说明文本投影无效。");
  return content.map((part: unknown) => {
    if (!part || typeof part !== "object" || !("type" in part) || part.type !== "text"
      || !("text" in part) || typeof part.text !== "string")
      throw new Error("note_caption_projection_invalid：说明只能包含纯文本。");
    return part.text;
  }).join("");
}

export function withCaptionBlocks(blocks: readonly PortableEditorBlock[]): PortableEditorBlock[] {
  return blocks.map(block => {
    const children = withCaptionBlocks(block.children ?? []);
    if (!hasBlockCaption(block.type)) return {...block, children};
    if (children.some(child => child.type === BLOCK_CAPTION)) return {...block, children};
    if (!String(block.props?.caption ?? "")) return {...block, children};
    return {...block, children: [{id: captionBlockId(block.id), type: BLOCK_CAPTION,
      props: {ownerId: block.id}, content: String(block.props?.caption ?? ""), children: []}, ...children]};
  });
}

export function withoutCaptionBlocks(blocks: readonly PortableEditorBlock[]): PortableEditorBlock[] {
  return blocks.map(block => {
    if (block.type === BLOCK_CAPTION) throw new Error("note_caption_owner_missing：说明必须归属于图片或表格。");
    const caption = block.children?.find(child => child.type === BLOCK_CAPTION);
    if (caption && (!hasBlockCaption(block.type) || caption.id !== captionBlockId(block.id)
      || caption.props?.ownerId !== block.id || caption.children?.length
      || block.children?.filter(child => child.type === BLOCK_CAPTION).length !== 1))
      throw new Error("note_caption_owner_invalid：说明归属无效。");
    const text = caption ? captionBlockText(caption.content) : undefined;
    return {...block, ...(text !== undefined && (text !== "" || Object.hasOwn(block.props ?? {}, "caption")) ? {props: {...block.props, caption: text}} : {}),
      children: withoutCaptionBlocks((block.children ?? []).filter(child => child.type !== BLOCK_CAPTION))};
  });
}
