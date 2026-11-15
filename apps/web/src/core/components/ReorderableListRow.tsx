import { GripVertical } from "lucide-react";
import {forwardRef, type ComponentPropsWithoutRef} from "react";
import { ReorderItem, type ReorderItemProps } from "./reorder/ReorderGroup";
import "./ReorderableListRow.css";

/** Presentation only: each host retains its ordering, locks and persistence. */
export function ReorderableListRow({ dropTarget = false, className = "", ...props }:
  Omit<ReorderItemProps,"as"> & { readonly dropTarget?: boolean }): React.JSX.Element {
  return <ReorderItem {...props} as="li" className={`reorderable-list-row ${className}${dropTarget ? " is-drop-target" : ""}`} />;
}

export const ReorderDragHandle = forwardRef<HTMLButtonElement, ComponentPropsWithoutRef<"button"> & {readonly compact?: boolean}>(
  function ReorderDragHandle({className = "", compact = false, ...props}, ref) {
    return <button {...props} ref={ref} type="button" draggable={false} className={`reorderable-drag-handle ${compact ? "is-compact" : ""} ${className}`}><GripVertical aria-hidden="true" size={16} /></button>;
  },
);
