import type { CSSProperties, ReactNode } from "react";
import { readingFieldColor } from "./structuredFieldColors";
/** Shared production card container. Editors and review clients supply content
 * and commands; layout and field identity have one owner. */
export function StructuredFieldCard({ id, title, color, pending, onFocus, indicators, actions, children, appearance = "plain" }: {
  readonly id: string; readonly title: string; readonly color?: string; readonly pending: boolean;
  readonly onFocus?: () => void; readonly indicators?: ReactNode; readonly actions?: ReactNode; readonly children: ReactNode;
  readonly appearance?: "plain" | "reading";
}): React.JSX.Element {
  return <article className="reading-category-card structured-resource-field" tabIndex={0}
    style={{ "--reading-field-color": readingFieldColor(id.replace(/^\$retained:/u, ""), color) } as CSSProperties}
    data-field-id={id} data-pending={pending} data-appearance={appearance} onFocus={onFocus}>
    <header><div>{indicators}<b>{title}</b></div><div>{actions}</div></header>{children}
  </article>;
}
