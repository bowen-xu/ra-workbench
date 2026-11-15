import { forwardRef, type ComponentPropsWithoutRef } from "react";
import { Plus } from "lucide-react";
import { ReorderDragHandle } from "../ReorderableListRow";
import "./BlockControls.css";

/** Shared gutter controls. Hosts supply commands, menus and gesture callbacks. */
export const BlockControls = forwardRef<HTMLDivElement, ComponentPropsWithoutRef<"div">>(function BlockControls({className = "", ...props}, ref) {
  return <div {...props} ref={ref} className={`block-controls ${className}`} />;
});
export const BlockAddButton = forwardRef<HTMLButtonElement, ComponentPropsWithoutRef<"button">>(function BlockAddButton({className = "", children, ...props}, ref) {
  return <button {...props} ref={ref} type="button" className={`block-control-button block-add-button ${className}`}><Plus aria-hidden="true" size={18}/>{children}</button>;
});
export const BlockDragButton = forwardRef<HTMLButtonElement, ComponentPropsWithoutRef<"button">>(function BlockDragButton({className = "", ...props}, ref) {
  return <ReorderDragHandle {...props} ref={ref} className={`block-control-button block-drag-button ${className}`} />;
});
