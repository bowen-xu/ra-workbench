/** A fixed title projects the title field of the canonical note document. */
export const DOCUMENT_TITLE = "documentTitle";
export const isDocumentTitle = (block: {readonly type?: string}) => block.type === DOCUMENT_TITLE;
export function documentBodyBlocks<T extends {readonly type?: string}>(blocks: readonly T[]): T[] {
  return blocks.filter(block => !isDocumentTitle(block));
}
