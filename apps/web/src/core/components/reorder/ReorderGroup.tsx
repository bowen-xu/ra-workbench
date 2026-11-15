import { Component, createRef, type HTMLAttributes, type CSSProperties, type Ref, type RefObject } from "react";
import { animateReorderMotion, cancelReorderMotion, type ReorderAxis } from "./reorderMotion";
import "./ReorderGroup.css";

interface GroupProps extends HTMLAttributes<HTMLElement> {
  readonly as?: "div" | "ol";
  readonly motion?: "default" | "none";
  readonly axis?: ReorderAxis;
  readonly order: readonly string[];
  readonly dragging?: boolean;
  readonly containerRef?: RefObject<HTMLElement | null>;
}
type Snapshot = ReadonlyMap<string, number> | null;

/** Capture pixels before React moves keyed children. A layout effect alone is
 * too late for asynchronous saves or unrelated authoritative order updates.
 * https://react.dev/reference/react/Component#getsnapshotbeforeupdate
 */
export class ReorderGroup extends Component<GroupProps, Record<string, never>, Snapshot> {
  private readonly element = createRef<HTMLElement>();
  private readonly animated = new Set<HTMLElement>();
  private rows(): HTMLElement[] {
    return this.element.current ? [...this.element.current.children].filter((row): row is HTMLElement => row instanceof HTMLElement && row.hasAttribute("data-reorder-id")) : [];
  }
  override getSnapshotBeforeUpdate(previous: GroupProps): Snapshot {
    if (this.props.motion === "none") return null;
    if (previous.order.join("\0") === this.props.order.join("\0") && !(previous.dragging && !this.props.dragging)) return null;
    const axis = this.props.axis ?? "vertical";
    return new Map(this.rows().map(row => {
      const rect = row.getBoundingClientRect();
      return [row.dataset.reorderId!, axis === "horizontal" ? rect.left : rect.top];
    }));
  }
  override componentDidUpdate(previous: GroupProps, _state: Record<string, never>, snapshot: Snapshot): void {
    if (!previous.dragging && this.props.dragging) cancelReorderMotion(this.animated);
    if (!snapshot || !this.element.current) return;
    cancelReorderMotion(this.animated); this.animated.clear();
    const style = getComputedStyle(this.element.current);
    const duration = Number.parseFloat(style.getPropertyValue("--reorder-motion-duration") || "180ms");
    const easing = style.getPropertyValue("--reorder-motion-easing").trim() || "cubic-bezier(.2,.8,.2,1)";
    for (const row of this.rows()) {
      const before = snapshot.get(row.dataset.reorderId!);
      if (before === undefined) continue;
      const rect = row.getBoundingClientRect();
      const offset = before - (this.props.axis === "horizontal" ? rect.left : rect.top);
      animateReorderMotion(row, offset, this.props.axis ?? "vertical", duration, easing);
      this.animated.add(row);
    }
  }
  override componentWillUnmount(): void { cancelReorderMotion(this.animated); }
  override render(): React.JSX.Element {
    const { as: Tag = "div", axis = "vertical", order: _order, dragging = false, motion = "default", containerRef, className = "", ...props } = this.props;
    return <Tag {...props} ref={(element: HTMLElement | null) => {
      this.element.current = element;
      if (containerRef) containerRef.current = element;
    }} className={`reorder-group ${className}${dragging ? " is-reordering" : ""}`} data-reorder-motion={motion} data-reorder-axis={axis} data-reordering={dragging || undefined} />;
  }
}

export interface ReorderItemProps extends HTMLAttributes<HTMLElement> {
  readonly as?: "div" | "li";
  readonly ref?: Ref<HTMLElement>;
  readonly itemId: string;
  readonly axis?: ReorderAxis;
  readonly dragging?: boolean;
  readonly offset?: number;
  readonly fixed?: boolean;
}
export function ReorderItem({ as: Tag = "div", ref, itemId, axis = "vertical", dragging = false, offset = 0, fixed = false, className = "", style, ...props }: ReorderItemProps): React.JSX.Element {
  return <Tag {...props} ref={(element: HTMLElement | null) => {
    if (typeof ref === "function") return ref(element);
    if (ref) ref.current = element;
  }} data-reorder-id={fixed ? undefined : itemId} data-dragging={dragging || undefined}
    className={`reorder-item ${className}${dragging ? " is-dragging" : ""}${offset ? " has-drag-offset" : ""}`}
    style={{ ...style, "--reorder-x": `${axis === "horizontal" ? offset : 0}px`, "--reorder-y": `${axis === "vertical" ? offset : 0}px` } as CSSProperties} />;
}
