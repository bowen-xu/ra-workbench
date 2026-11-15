export interface TextBlockConstraint {
  readonly fixed?: boolean;
  readonly splitType?: string;
  readonly maxLength?: number;
  readonly singleLine?: boolean;
  readonly plainText?: boolean;
  /** Attached text splits into the owning block's surrounding group. */
  readonly splitOutsideParent?: boolean;
}
export type TextBlockConstraints = Readonly<Record<string, TextBlockConstraint>>;
/** Shared capability declarations for editing, movement and hit testing. */
export const blockEditingConstraints: TextBlockConstraints = {
  documentTitle: {fixed: true, splitType: "paragraph", maxLength: 240, singleLine: true, plainText: true},
  blockCaption: {fixed: true, splitType: "paragraph", maxLength: 4000, plainText: true, splitOutsideParent: true},
};
export const isFixedBlock = (block: {readonly type?: string}) => !!blockEditingConstraints[block.type ?? ""]?.fixed;
export const isAttachedTextBlock = (block: {readonly type?: string}) => !!blockEditingConstraints[block.type ?? ""]?.splitOutsideParent;
