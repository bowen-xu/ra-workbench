import {useLayoutEffect} from "react";
import "./ApplicationScrollbars.css";

import {isOverNativeScrollbar} from "./nativeScrollbarHitTest";

interface ScrollbarActivity {timer: number | undefined; animation: Animation | undefined}
const alpha = "--ui-scrollbar-alpha";
const hover = "--ui-scrollbar-hover";

/** Own paint visibility for every native scroller in one document, including
 * portal content. Never change overflow, geometry, scroll position or input. */
export function installApplicationScrollbars(owner: Document): () => void {
  const ownerView = owner.defaultView;
  if (!ownerView) return () => {};
  const view = ownerView;
  const activities = new Map<HTMLElement, ScrollbarActivity>();
  let hovered: HTMLElement | undefined;
  let dragged: HTMLElement | undefined;
  const reducedMotion = view.matchMedia?.("(prefers-reduced-motion: reduce)");
  owner.documentElement.dataset.appScrollbars = "true";
  function cancelAnimation(activity: ScrollbarActivity): void {
    if (!activity.animation) return;
    activity.animation.onfinish = null;
    activity.animation.cancel(); activity.animation = undefined;
  }
  function hide(node: HTMLElement, activity: ScrollbarActivity): void {
    if (activities.get(node) !== activity) return;
    cancelAnimation(activity);
    node.style.removeProperty(alpha); node.style.removeProperty(hover); activities.delete(node);
  }
  function show(node: HTMLElement): ScrollbarActivity {
    const activity = activities.get(node) ?? {timer: undefined, animation: undefined};
    if (activity.timer !== undefined) view.clearTimeout(activity.timer);
    activity.timer = undefined;
    cancelAnimation(activity);
    activities.set(node, activity); node.style.setProperty(alpha, "1");
    return activity;
  }
  function scheduleHide(node: HTMLElement, activity: ScrollbarActivity): void {
    if (node === hovered || node === dragged) return;
    activity.timer = view.setTimeout(() => {
      activity.timer = undefined;
      if (!node.isConnected || reducedMotion?.matches || typeof node.animate !== "function") {hide(node, activity); return;}
      // Animate the registered value on the REAL element. WebKit's scrollbar
      // pseudo elements do not support ordinary CSS transitions. WAAPI also
      // leaves each component's existing transitions / animations intact.
      const animation = node.animate({[alpha]: ["1", "0"]}, {duration: 200, easing: "ease-out", fill: "forwards"});
      activity.animation = animation;
      animation.onfinish = () => {if (activity.animation === animation) hide(node, activity);};
    }, 700);
  }
  function onScroll(event: Event): void {
    const node = event.target === owner ? owner.scrollingElement : event.target;
    if (node instanceof HTMLElement) scheduleHide(node, show(node));
  }
  function setHovered(node: HTMLElement | undefined): void {
    if (hovered === node) return;
    const previous = hovered;
    hovered = node;
    if (previous) {
      previous.style.removeProperty(hover);
      scheduleHide(previous, show(previous));
    }
    if (node) {show(node); node.style.setProperty(hover, "1");}
  }
  function onPointerMove(event: PointerEvent): void {
    if (event.pointerType === "touch") return;
    let node = event.target instanceof HTMLElement ? event.target : undefined;
    while (node && !isOverNativeScrollbar(node, event.clientX, event.clientY)) node = node.parentElement ?? undefined;
    setHovered(node);
  }
  function onPointerOut(event: PointerEvent): void {
    if (!event.relatedTarget) setHovered(undefined);
  }
  function onPointerDown(event: PointerEvent): void {
    if (event.pointerType === "touch") setHovered(undefined);
    else onPointerMove(event);
    dragged = hovered;
  }
  function onPointerUp(): void {
    const previous = dragged;
    dragged = undefined;
    if (previous) scheduleHide(previous, show(previous));
  }
  function onBlur(): void {setHovered(undefined); onPointerUp();}
  // Native scroll does not bubble. Capture reveals only the actual owner,
  // rather than ancestors of a scrolling tool/history/formula viewport.
  owner.addEventListener("scroll", onScroll, {capture: true, passive: true});
  owner.addEventListener("pointermove", onPointerMove, {capture: true, passive: true});
  owner.addEventListener("pointerout", onPointerOut, {capture: true, passive: true});
  owner.addEventListener("pointerdown", onPointerDown, {capture: true, passive: true});
  owner.addEventListener("pointerup", onPointerUp, {capture: true, passive: true});
  owner.addEventListener("pointercancel", onBlur, {capture: true, passive: true});
  view.addEventListener("blur", onBlur);
  return () => {
    owner.removeEventListener("scroll", onScroll, true);
    owner.removeEventListener("pointermove", onPointerMove, true);
    owner.removeEventListener("pointerout", onPointerOut, true);
    owner.removeEventListener("pointerdown", onPointerDown, true);
    owner.removeEventListener("pointerup", onPointerUp, true);
    owner.removeEventListener("pointercancel", onBlur, true);
    view.removeEventListener("blur", onBlur);
    for (const [node, activity] of activities) {
      if (activity.timer !== undefined) view.clearTimeout(activity.timer);
      hide(node, activity);
    }
    delete owner.documentElement.dataset.appScrollbars;
  };
}

export function ApplicationScrollbars(): null {
  useLayoutEffect(() => installApplicationScrollbars(document), []);
  return null;
}
