import type { InputHTMLAttributes } from "react";
import "./Switch.css";

export function Switch(props: Omit<InputHTMLAttributes<HTMLInputElement>, "type" | "role">): React.JSX.Element {
  return <input {...props} type="checkbox" role="switch" className={`ui-switch ${props.className ?? ""}`} />;
}

/** Menu rows own keyboard and checked semantics; their switch is decorative. */
export function SwitchIndicator({checked}: {readonly checked: boolean}): React.JSX.Element {
  return <span className="ui-switch" data-checked={checked} aria-hidden="true"/>;
}
