import { ChevronDown } from "lucide-react";
import type { SelectHTMLAttributes } from "react";

/** Native keyboard/selection semantics, with width derived from the current label. */
export function SettingsSelect({ options, value, ...props }: Omit<SelectHTMLAttributes<HTMLSelectElement>, "children"> & {
  readonly options: readonly { readonly value: string; readonly label: string }[];
}): React.JSX.Element {
  return <span className="settings-select"><span aria-hidden="true">{options.find(option => option.value === value)?.label ?? ""}</span>
    <select {...props} value={value}>{options.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</select>
    <ChevronDown size={15} aria-hidden="true" />
  </span>;
}
