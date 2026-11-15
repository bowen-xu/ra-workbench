import { useRef, useState } from "react";
import { FileText } from "lucide-react";
import { ControlHint } from "../ControlHint";
import { useVisibleLayoutMeasurement } from "../layout/useVisibleLayoutMeasurement";

export function TextEditorPath({ path }: { readonly path: string }): React.JSX.Element {
  const tailRef = useRef<HTMLSpanElement>(null);
  const [truncated, setTruncated] = useState(false);
  useVisibleLayoutMeasurement(() => tailRef.current, element => setTruncated(element.scrollWidth > element.clientWidth));
  const parts = path.split(/[\\/]/).filter(Boolean);
  const root = parts.length > 1 ? parts[0] : undefined;
  const tail = (root === undefined ? parts : parts.slice(1)).join(" › ");
  const anchor = <span className="text-editor-path" tabIndex={0} aria-label={path}>
    <FileText size={16} aria-hidden="true" />
    {root && <><span className="text-path-root">{root}</span><span className="text-path-separator" aria-hidden="true">›</span></>}
    <span className="text-path-tail" ref={tailRef}><bdi dir="ltr">{tail}</bdi></span>
  </span>;
  return truncated ? <ControlHint content={path}>{anchor}</ControlHint> : anchor;
}
