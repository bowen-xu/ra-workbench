import { ControlHint } from "../ControlHint";
import type { ReactNode } from "react";
import { Fragment } from "react";
import "./EditorDiffControls.css";
import type { EditorDiffHighlightAppearance } from "./EditorDiffHighlight";

export type EditorReviewAction = "undo" | "incoming" | "both" | "keep";
export interface EditorReviewConfiguration {
  readonly highlight?: EditorDiffHighlightAppearance;
  /** Default is the frozen reference's Take + Both. An empty list keeps only
   * Undo/Keep; hiding a choice does not mark a conflict as resolved. */
  readonly conflictActions?: readonly ("incoming" | "both")[];
  readonly labels?: Partial<Readonly<Record<EditorReviewAction, string>>>;
  readonly hints?: Partial<Readonly<Record<EditorReviewAction, string>>>;
  readonly showBulk?: boolean;
  /** Read-only content slot. Hosts must not install a second editable branch. */
  readonly renderIncoming?: (text: string) => ReactNode;
}
const labels = { undo: "Undo", incoming: "Take", both: "Both", keep: "Keep" };
const hints: Partial<Record<EditorReviewAction, string>> = { incoming: "Use incoming", both: "Use both" };

/** Undo/Keep and their optional conflict choices are one review surface.
 * This component emits intent; the editor/history/authority own the operation. */
export function EditorReviewActionGroup({ onResolve, conflict = true, disabled = false, configuration }: {
  readonly onResolve: (action: EditorReviewAction) => void;
  readonly conflict?: boolean;
  readonly disabled?: boolean;
  readonly configuration?: EditorReviewConfiguration | undefined;
}): React.JSX.Element {
  const choices: readonly ("incoming" | "both")[] = conflict ? [...new Set(configuration?.conflictActions ?? ["incoming", "both"] as const)] : [];
  const actions: readonly EditorReviewAction[] = ["undo", ...choices, "keep"];
  return <div role="group" aria-label={conflict ? "冲突变更操作" : "变更操作"}>
    {actions.map(action => {
      const hint = configuration?.hints?.[action] ?? hints[action];
      const button = <button className={`writing-diff-chunk-${action === "both" ? "incoming" : action}`} disabled={disabled} type="button"
        onClick={() => onResolve(action)}>{configuration?.labels?.[action] ?? labels[action]}</button>;
      return hint ? <ControlHint key={action} content={hint}>{button}</ControlHint> : <Fragment key={action}>{button}</Fragment>;
    })}
  </div>;
}
