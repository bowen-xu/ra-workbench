import { ControlHint } from "./ControlHint";
import { useRef, useState } from "react";
import { useVisibleLayoutMeasurement } from "./layout/useVisibleLayoutMeasurement";

export interface DoclingNotificationDocument {
  readonly paperTitle: string;
  readonly fileName: string;
}

/** Measure rendered typography so Chinese, Latin and narrow panels share the same truncation rule. */
function CaptionLine({ text, middle = false }: { readonly text: string; readonly middle?: boolean }): React.JSX.Element {
  const ref = useRef<HTMLSpanElement>(null);
  const [visible, setVisible] = useState(text);
  const canvas = useRef<CanvasRenderingContext2D | null | undefined>(undefined);
  useVisibleLayoutMeasurement(() => ref.current, element => {
    if (canvas.current === undefined) canvas.current = document.createElement("canvas").getContext("2d");
    const context = canvas.current;
    if (!context) return;
    context.font = getComputedStyle(element).font;
    const width = element.clientWidth;
    if (context.measureText(text).width <= width) { setVisible(text); return; }
    const characters = Array.from(text);
    const candidate = (count: number) => middle
      ? `${characters.slice(0, Math.ceil(count / 2)).join("")}...${count > 1 ? characters.slice(-Math.floor(count / 2)).join("") : ""}`
      : `${characters.slice(0, count).join("")}...`;
    let low = 0;
    let high = characters.length;
    while (low < high) {
      const count = Math.ceil((low + high) / 2);
      if (context.measureText(candidate(count)).width <= width) low = count;
      else high = count - 1;
    }
    setVisible(candidate(low));
  });
  return <ControlHint content={text}><span className="docling-notification-caption-line" ref={ref} aria-label={text}>{visible}</span></ControlHint>;
}

export function DoclingNotificationCaption({ paperTitle, fileName }: DoclingNotificationDocument): React.JSX.Element {
  return <span className="docling-notification-caption">
    <CaptionLine text={paperTitle} />
    <CaptionLine text={fileName} middle />
  </span>;
}
