import type { ComponentPropsWithRef, ElementType, ReactNode, Ref } from "react";
import "./InlineReference.css";

interface Presentation {
  readonly label: string;
  readonly icon?: ReactNode;
  readonly truncate?: boolean;
  readonly tone?: "accent" | "inherit";
  readonly underline?: "none" | "dashed";
  readonly labelRef?: Ref<HTMLSpanElement>;
  readonly labelContent?: ReactNode;
}

/** Configurable presentation; the host retains identity and editor content ownership. */
export function InlineReference<T extends ElementType = "span">({
  as, label, icon, truncate = true, tone = "accent", underline = "none", labelRef, labelContent, className, ...props
}: Presentation & { readonly as?: T } & Omit<ComponentPropsWithRef<T>, keyof Presentation | "as" | "children">): React.JSX.Element {
  const Tag: ElementType = as ?? "span";
  return <Tag {...props} className={`inline-reference${className ? ` ${className}` : ""}`}
    data-truncate={truncate} data-tone={tone} data-underline={underline}
    title={props.title ?? label} aria-label={props["aria-label"] ?? label}>
    {icon && <span className="inline-reference-icon" contentEditable={false}>{icon}</span>}
    <span ref={labelRef} className="inline-reference-label agent-composer-reference-label">{labelRef ? labelContent : labelContent ?? label}</span>
  </Tag>;
}
