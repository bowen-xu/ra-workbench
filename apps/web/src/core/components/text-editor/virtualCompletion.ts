import { autocompletion } from "@codemirror/autocomplete";
import type { Extension } from "@codemirror/state";
import { completionIcon } from "./completionIcon";
import "./virtualCompletion.css";

/** Shared CM completion engine with the pinned VS Code list/row-cache adapter. */
export function virtualCompletion(config: NonNullable<Parameters<typeof autocompletion>[0]>): Extension {
  return autocompletion({ ...config, virtualize: true, icons: false,
    addToOptions: [...(config.addToOptions ?? []), ...(config.icons === false ? [] : [{
      position: 20,
      render: completionIcon,
    }])],
  });
}
