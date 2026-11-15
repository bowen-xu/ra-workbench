import { RotateCcw } from "lucide-react";
import { useAppRuntime } from "../runtime/AppRuntimeProvider";
import { ControlHint } from "./ControlHint";
import "./PreviewResetControl.css";
import { notify } from "../services/notifications";

/** The App owns this control; the runtime supplies the isolated reset capability. */
export function PreviewResetControl(): React.JSX.Element | null {
  const { preview } = useAppRuntime();
  if (!preview) return null;
  return <ControlHint content="重置预览数据，清除模拟编辑">
    <button className="preview-reset-control" aria-label="重置预览数据" type="button"
      onClick={() => { void Promise.resolve().then(() => preview.reset()).catch(reason => notify.error(reason instanceof Error ? reason.message : String(reason))); }}>
      <RotateCcw aria-hidden="true" size={13} strokeWidth={1.7} />
    </button>
  </ControlHint>;
}
