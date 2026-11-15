import type { RootContent } from "hast";

export interface MarkdownInput { readonly value: string; readonly reveal: boolean; readonly streaming: boolean; }
export type MarkdownRequest = { readonly kind: "render"; readonly owner: number; readonly revision: number; readonly input: MarkdownInput }
  | { readonly kind: "release"; readonly owner: number };
/** An absent tree means reuse the previous block at this index, not an empty block. */
export interface MarkdownBlockPatch { readonly revision: number; readonly tree?: RootContent; }
export type MarkdownReply = { readonly owner: number; readonly revision: number } & (
  { readonly kind: "rendered"; readonly blocks: readonly MarkdownBlockPatch[] }
  | { readonly kind: "failed"; readonly code: "AGENT_MARKDOWN_RENDER_FAILED" });
export interface MarkdownBlock { readonly revision: number; readonly tree: RootContent; }
