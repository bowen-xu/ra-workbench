import type { ReactNode } from "react";
import { Info } from "lucide-react";
import { ControlHint } from "../../components/ControlHint";

export function SettingRow({ title, description, details, children }: {
  readonly title: ReactNode;
  readonly description?: string;
  readonly details?: ReactNode;
  readonly children: ReactNode;
}): React.JSX.Element {
  return <div className="setting-row">
    <div className="setting-copy"><span className="setting-title">{title}</span>
      {description && <p>{description}</p>}{details && <div className="setting-details">{details}</div>}</div>
    <div className="setting-control">{children}</div>
  </div>;
}

export function SettingsHint({ label, children }: { readonly label: string; readonly children: ReactNode }): React.JSX.Element {
  return <ControlHint content={children}><button className="settings-info" type="button" aria-label={label}><Info size={15} aria-hidden="true" /></button></ControlHint>;
}

export function SettingField({ label, description, children }: { readonly label: string; readonly description?: string; readonly children: ReactNode }): React.JSX.Element {
  return <div className="setting-field" role="group" aria-label={label}><span className="setting-title">{label}</span>
    {description && <p>{description}</p>}{children}</div>;
}

export function SettingGroup({ label, children }: { readonly label: string; readonly children: ReactNode }): React.JSX.Element {
  return <div className="setting-group" role="group" aria-label={label}>{children}</div>;
}
