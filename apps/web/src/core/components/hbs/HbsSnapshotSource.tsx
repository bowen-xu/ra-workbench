import "./HbsSnapshotSource.css";

/** Diagnostic snapshots show the stored value, without an editor or renderer. */
export function HbsSnapshotSource({ value, text = false }: { readonly value: unknown; readonly text?: boolean }): React.JSX.Element {
  return <pre className="hbs-snapshot-source">{text ? String(value) : JSON.stringify(value, null, 2)}</pre>;
}
