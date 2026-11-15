import {WheelGestures, type WheelEventData, type WheelEventState} from "wheel-gestures";

/** Shared active/momentum classifier. A quiet period alone never grants momentum
 * permission to change scroll owners or pages. Native phases take precedence. */
export function createWheelGestureIntent() {
  let coasting = false, momentumCancelled = false, pendingActive = false;
  let sample: WheelEventState | undefined;
  const gestures = WheelGestures({preventWheelAction: false, reverseSign: false});
  const unsubscribe = gestures.on("wheel", state => {
    if (state.isEnding) { if (state.isMomentumCancel) momentumCancelled = true; return; }
    sample = state;
  });
  return {
    feed(event: WheelEvent): {readonly momentum: boolean; readonly restarted: boolean; readonly start: boolean} {
      const nativeMomentum = (event as WheelEventData).momentum;
      if (event.deltaX === 0 && event.deltaY === 0) {
        const wasCoasting = coasting; momentumCancelled = false;
        gestures.feedWheel(event);
        pendingActive ||= momentumCancelled || (wasCoasting && nativeMomentum === false);
        if (typeof nativeMomentum === "boolean") coasting = nativeMomentum;
        return {momentum: coasting, restarted: false, start: false};
      }
      const previous = sample; momentumCancelled = false; gestures.feedWheel(event);
      const magnitude = Math.abs(event.deltaY), previousMagnitude = Math.abs(previous?.axisDelta[1] ?? 0);
      const renewed = coasting && nativeMomentum !== true && (nativeMomentum === false || magnitude > Math.max(2, previousMagnitude * 2));
      const newActiveStream = !coasting && nativeMomentum !== true && sample !== previous && sample?.isStart === true && !!previous && magnitude >= previousMagnitude;
      const restarted = pendingActive || momentumCancelled || renewed || event.deltaMode !== 0 || newActiveStream;
      pendingActive = false;
      coasting = typeof nativeMomentum === "boolean" ? nativeMomentum : restarted ? false : coasting || sample?.isMomentum === true;
      return {momentum: coasting, restarted, start: !previous || restarted || (!coasting && sample !== previous && sample?.isStart === true)};
    },
    dispose(): void { unsubscribe(); gestures.disconnect(); },
  };
}
