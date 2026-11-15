import {FilePenLine, LibraryBig, NotebookText, Settings2} from "lucide-react";
import type {ReactNode} from "react";
import type {WorkbenchComposition} from "../workbenchComposition";
import type {RegisteredPluginView} from "../../core/runtime/plugins/viewRegistry";

/** Presentation order is independent of registration and activation order. */
export function workbenchNavigationItems(_composition: Pick<WorkbenchComposition, "plugins">, registered: readonly RegisteredPluginView[]) {
  const icons: Readonly<Record<string, ReactNode>> = {
    library: <LibraryBig size={20} strokeWidth={1.8}/>,
    notes: <NotebookText size={20} strokeWidth={1.8}/>,
    writing: <FilePenLine size={20} strokeWidth={1.8}/>,
    settings: <Settings2 size={20} strokeWidth={1.8}/>,
  };
  const items = [
    ...registered.map(view => ({
      id: view.id, label: view.title, priority: view.priority ?? 0,
      icon: icons[view.id] ?? <NotebookText size={20} strokeWidth={1.8}/>,
    })),
    {id: "settings" as const, label: "设置", priority: -1000, icon: icons.settings},
  ];
  return items.sort((left, right) => right.priority - left.priority || left.id.localeCompare(right.id, "en"));
}
