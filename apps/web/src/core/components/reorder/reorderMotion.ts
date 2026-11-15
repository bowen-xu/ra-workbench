export type ReorderAxis = "horizontal" | "vertical";

const animations = new WeakMap<HTMLElement, Animation>();

export function cancelReorderMotion(elements: Iterable<HTMLElement>): void {
  for (const element of elements) {
    animations.get(element)?.cancel();
    animations.delete(element);
  }
}

export function animateReorderMotion(element: HTMLElement, offset: number, axis: ReorderAxis, duration: number, easing: string): void {
  if (Math.abs(offset) < .5 || duration <= 0) return;
  const translate = axis === "horizontal" ? "translateX" : "translateY";
  animations.set(element, element.animate([
    { transform: `${translate}(${offset}px)` }, { transform: `${translate}(0px)` },
  ], { duration, easing }));
}
