export type EditorReviewLocationSide = "before" | "after";

/** Location navigation is separate from the one original review decision. */
export function editorReviewLocationActions(side: EditorReviewLocationSide): {
  readonly jumpLabel: "old" | "new";
  readonly jumpHint: "jump to old" | "jump to new";
  readonly resolve: "keep" | "undo";
} {
  return side === "before" ? {jumpLabel:"new",jumpHint:"jump to new",resolve:"undo"}
    : {jumpLabel:"old",jumpHint:"jump to old",resolve:"keep"};
}
