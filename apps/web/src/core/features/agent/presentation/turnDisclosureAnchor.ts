interface TurnDisclosureAnchor {
  readonly restore: () => void;
  readonly release: () => void;
}

const reservations = new WeakMap<HTMLElement, () => void>();

/** Release a preserved scroll range before an external return-to-bottom action. */
export function releaseTurnDisclosureAnchor(scroller: HTMLElement): void {
  reservations.get(scroller)?.();
}

/** Keep a manually toggled summary stationary, including near the scroll limit. */
export function captureTurnDisclosureAnchor(summary: HTMLElement): TurnDisclosureAnchor | undefined {
  const scroller = summary.closest<HTMLElement>(".writing-agent-messages");
  const transcript = summary.closest<HTMLElement>(".writing-agent-transcript");
  if (!scroller || !transcript) return;
  const top = summary.getBoundingClientRect().top;
  let expectedScrollTop = scroller.scrollTop;
  // Retain only enough height to prevent collapse from clamping scrollTop.
  // Keep beyond the panel's six-pixel bottom-follow threshold as well.
  const minimumHeight = expectedScrollTop + scroller.clientHeight + 8
    - (scroller.scrollHeight - transcript.offsetHeight);
  reservations.get(scroller)?.();
  const previousMinimum = transcript.style.minHeight;
  transcript.style.minHeight = `max(100%, ${Math.max(0, minimumHeight)}px)`;
  scroller.dispatchEvent(new Event("writing-agent-preserve-position"));

  const release = () => {
    if (reservations.get(scroller) !== release) return;
    reservations.delete(scroller);
    transcript.style.minHeight = previousMinimum;
    scroller.removeEventListener("wheel", release);
    scroller.removeEventListener("touchstart", release);
    scroller.removeEventListener("scroll", onScroll);
    scroller.removeEventListener("keydown", onKeyDown);
  };
  const onScroll = () => {
    if (Math.abs(scroller.scrollTop - expectedScrollTop) > 1) release();
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End"].includes(event.key)) release();
  };
  reservations.set(scroller, release);
  scroller.addEventListener("wheel", release, { passive: true });
  scroller.addEventListener("touchstart", release, { passive: true });
  scroller.addEventListener("scroll", onScroll, { passive: true });
  scroller.addEventListener("keydown", onKeyDown);
  let restored = false;
  return {
    restore: () => {
      if (restored) return;
      restored = true;
      expectedScrollTop += summary.getBoundingClientRect().top - top;
      scroller.scrollTop = expectedScrollTop;
    },
    release,
  };
}
