export type ImageAlignment = "left" | "center" | "right";

/** Picture silhouettes share the same line weight and geometry. */
export function ImageAlignmentIcon({alignment}: {readonly alignment: ImageAlignment}): React.JSX.Element {
  const x = alignment === "left" ? 3 : alignment === "right" ? 11 : 7;
  return <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"
    strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <path d="M2 3.5h20M2 20.5h20"/>
    <rect x={x} y="7" width="10" height="10" rx="1.5"/>
  </svg>;
}

export function ImageCaptionIcon(): React.JSX.Element {
  return <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"
    strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <rect x="6" y="3" width="12" height="10" rx="1.5"/>
    <path d="M3 17.5h18M6 21h12"/>
  </svg>;
}
