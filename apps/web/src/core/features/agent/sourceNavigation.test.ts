/** @vitest-environment happy-dom */
import { expect, it, vi } from "vitest";
import { navigateSource, SOURCE_NAVIGATION_EVENT, type SourceNavigationRequest } from "./sourceNavigation";
it("bounds a closed or unavailable editor target and clears the deadline after acknowledgment",async()=>{
  vi.useFakeTimers();let request!:SourceNavigationRequest;
  const listen=(event:Event)=>{event.preventDefault();request=(event as CustomEvent<SourceNavigationRequest>).detail;};
  window.addEventListener(SOURCE_NAVIGATION_EVENT,listen);
  try{
    const stalled=navigateSource("source://notes/note:#block{b}",{locate:true});const rejected=expect(stalled).rejects.toThrow("请加载后重试");
    await vi.advanceTimersByTimeAsync(30_000);await rejected;
    const success=navigateSource("source://notes/note:#block{b}",{locate:true});const location={getAnchorRect:()=>new DOMRect(1,2,3,4)};
    request.resolve(location);expect(await success).toBe(location);expect(vi.getTimerCount()).toBe(0);
  }finally{window.removeEventListener(SOURCE_NAVIGATION_EVENT,listen);vi.useRealTimers();}
});
