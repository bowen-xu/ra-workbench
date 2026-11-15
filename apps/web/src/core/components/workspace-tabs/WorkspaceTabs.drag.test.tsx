// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { getAppStorage } from "../../runtime/appStorage";
import { WorkspaceTabBar, WorkspaceTabsProvider, usePublishWorkspaceTabs, type WorkspaceTab } from "./WorkspaceTabs";
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
let dispose:(()=>Promise<void>)|undefined;
beforeEach(() => getAppStorage().clear());
afterEach(async()=>{await dispose?.();dispose=undefined;document.body.innerHTML="";vi.restoreAllMocks();});
async function setup(){
  const select=vi.fn(),close=vi.fn();
  const initial:readonly WorkspaceTab[]=[
    {id:"home",home:true,title:"首页",owner:"library",active:false,select},
    ...["a","b","c"].map(id=>({id,title:id,owner:"library" as const,active:id==="a",select,close})),
  ];
  function Publish({tabs}:{readonly tabs:readonly WorkspaceTab[]}){usePublishWorkspaceTabs("library",tabs);return null;}
  function Shell({tabs=initial}:{readonly tabs?:readonly WorkspaceTab[]}){return <WorkspaceTabsProvider>
    <Publish tabs={tabs}/><WorkspaceTabBar owner="library" textActive={false} navigate={(_,__,ready)=>{ready();return true;}}/>
  </WorkspaceTabsProvider>;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<Shell/>));
  const rail=host.querySelector<HTMLDivElement>('[role="tablist"]')!;
  vi.spyOn(rail,"getBoundingClientRect").mockReturnValue({left:0,right:500,width:500} as DOMRect);
  // happy-dom permits negative scrollLeft during the initial zero-layout render.
  rail.scrollLeft=0;
  const frames=[{id:"home",left:0,width:30},{id:"a",left:36,width:80},{id:"b",left:122,width:140},{id:"c",left:268,width:100}];
  const rows=new Map(frames.map(frame=>[frame.id,host.querySelector<HTMLElement>(`[data-tab-id="${frame.id}"]`)!]));
  const animations=new Map(frames.map(frame=>{
    const cancel=vi.fn(),animate=vi.fn(()=>({cancel}));
    Object.defineProperty(rows.get(frame.id)!,"animate",{value:animate});
    return [frame.id,{animate,cancel}] as const;
  }));
  const measurements=new Map(frames.map(frame=>[frame.id,vi.spyOn(rows.get(frame.id)!,"getBoundingClientRect").mockReturnValue(new DOMRect(frame.left,0,frame.width,26))]));
  const button=(id:string)=>rows.get(id)!.querySelector<HTMLButtonElement>('[role="tab"]')!;
  const event=async(target:Element|Window,type:string,x:number,pointerId=1)=>act(async()=>{
    target.dispatchEvent(new PointerEvent(type,{button:0,isPrimary:true,pointerId,clientX:x,clientY:18,bubbles:true,cancelable:true}));
  });
  const order=()=>[...rail.querySelectorAll<HTMLElement>('[data-tab-id]')].map(row=>row.dataset.tabId);
  const offset=(id:string)=>rows.get(id)!.style.getPropertyValue("--reorder-x");
  return {host,rail,rows,button,event,order,offset,select,close,measurements,animations,
    replace:async(tabs:readonly WorkspaceTab[])=>act(async()=>root.render(<Shell tabs={tabs}/>)),initial};
}

it("previews variable-width displacement from static queue-style geometry and commits once without activating a tab",async()=>{
  const f=await setup(),a=f.rows.get("a"),b=f.rows.get("b");
  await f.event(f.button("a"),"pointerdown",60);
  await f.event(window,"pointermove",350);
  expect(f.order()).toEqual(["home","a","b","c"]);
  expect(f.offset("a")).toBe("252px");expect(f.offset("b")).toBe("-86px");expect(f.offset("c")).toBe("-86px");
  expect(f.rows.get("a")?.dataset.dragging).toBe("true");expect(f.offset("home")).toBe("0px");
  expect(f.host.querySelector("[data-drop]")).toBeNull();
  // Animated frames must never feed back into hit testing or displacement.
  f.measurements.get("b")!.mockReturnValue({left:36,width:140,right:176} as DOMRect);
  f.measurements.get("c")!.mockReturnValue({left:182,width:100,right:282} as DOMRect);
  await f.event(window,"pointermove",350);
  expect(f.offset("a")).toBe("252px");
  expect(f.measurements.get("b")).toHaveBeenCalledOnce();expect(f.measurements.get("c")).toHaveBeenCalledOnce();
  // A browser's compatibility click follows pointerup in the same task. A
  // later programmatic click is a separate user activation, not that click.
  await act(async()=>{
    window.dispatchEvent(new PointerEvent("pointerup",{button:0,isPrimary:true,pointerId:1,clientX:350,clientY:18,bubbles:true,cancelable:true}));
    f.button("a").click();
  });
  expect(f.order()).toEqual(["home","b","c","a"]);
  expect(f.host.querySelector('[data-tab-id="a"]')).toBe(a);expect(f.host.querySelector('[data-tab-id="b"]')).toBe(b);
  expect(f.offset("a")).toBe("0px");expect(f.select).not.toHaveBeenCalled();expect(f.close).not.toHaveBeenCalled();
  await act(async()=>f.button("a").click());expect(f.select).toHaveBeenCalledOnce();
});

it.each(["escape","pointercancel"])("clears the displacement preview without committing on %s",async cancellation=>{
  const f=await setup();await f.event(f.button("a"),"pointerdown",60);await f.event(window,"pointermove",350);
  if(cancellation==="escape")await act(async()=>window.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape"})));
  else await f.event(window,"pointercancel",350);
  expect(f.order()).toEqual(["home","a","b","c"]);expect(f.offset("a")).toBe("0px");
  expect(f.host.querySelector('[data-dragging="true"]')).toBeNull();
  expect(f.select).not.toHaveBeenCalled();expect(f.close).not.toHaveBeenCalled();
});

it("keeps clicks below the drag threshold and Home outside the reordering gesture",async()=>{
  const f=await setup();await f.event(f.button("a"),"pointerdown",60);await f.event(window,"pointermove",64);await f.event(window,"pointerup",64);
  expect(f.host.querySelector('[data-dragging="true"]')).toBeNull();
  await act(async()=>f.button("a").click());expect(f.select).toHaveBeenCalledOnce();
  await f.event(f.button("home"),"pointerdown",15);await f.event(window,"pointermove",350);await f.event(window,"pointerup",350);
  expect(f.order()).toEqual(["home","a","b","c"]);
});

it("uses content coordinates when the strip scrolls and cancels stale previews if a tab closes",async()=>{
  const f=await setup();await f.event(f.button("a"),"pointerdown",60);
  f.rail.scrollLeft=100;
  await f.event(window,"pointermove",250);expect(f.offset("a")).toBe("252px");
  await f.replace(f.initial.filter(tab=>tab.id!=="c"));
  expect(f.host.querySelector('[data-dragging="true"]')).toBeNull();expect(f.offset("a")).toBe("0px");
  await f.event(window,"pointerup",250);expect(f.order()).toEqual(["home","a","b"]);
});

it("switches at slot boundaries before neighboring centers and keeps a double-threshold deadband",async()=>{
  const f=await setup();await f.event(f.button("a"),"pointerdown",60);
  // A/B boundary is 119, well before B's center at 192.
  await f.event(window,"pointermove",123);expect(f.offset("a")).toBe("0px");
  await f.event(window,"pointermove",124);expect(f.offset("a")).toBe("146px");
  for(const x of [119,116,115,120,123]){
    await f.event(window,"pointermove",x);expect(f.offset("a")).toBe("146px");
  }
  await f.event(window,"pointermove",114);expect(f.offset("a")).toBe("0px");
  await f.event(window,"pointermove",124);
  await f.event(window,"pointerup",124);
  expect(f.order()).toEqual(["home","b","a","c"]);
  expect(f.select).not.toHaveBeenCalled();
});

it("crosses left boundaries with unequal widths and can skip several slots",async()=>{
  const f=await setup();await f.event(f.button("c"),"pointerdown",320);
  // B/C boundary is 265, well to the right of B's center at 192.
  await f.event(window,"pointermove",261);expect(f.offset("c")).toBe("0px");
  await f.event(window,"pointermove",260);expect(f.offset("c")).toBe("-146px");
  await f.event(window,"pointermove",269);expect(f.offset("c")).toBe("-146px");
  await f.event(window,"pointermove",270);expect(f.offset("c")).toBe("0px");
  await f.event(window,"pointermove",100);expect(f.offset("c")).toBe("-232px");
  await f.event(window,"pointerup",100);expect(f.order()).toEqual(["home","c","a","b"]);
});

it("applies boundary thresholds in content coordinates after scrolling",async()=>{
  const f=await setup();await f.event(f.button("a"),"pointerdown",60);
  f.rail.scrollLeft=50;
  expect(f.rail.scrollLeft).toBe(50);
  await f.event(window,"pointermove",73);expect(f.rail.scrollLeft).toBe(50);expect(f.offset("a")).toBe("0px");
  await f.event(window,"pointermove",74);expect(f.offset("a")).toBe("146px");
  await f.event(window,"pointerup",74);expect(f.order()).toEqual(["home","b","a","c"]);
});

it("rebases an unfinished preview onto the committed layout without applying displacement twice",async()=>{
  const f=await setup();await f.event(f.button("a"),"pointerdown",60);
  await f.event(window,"pointermove",124);
  // Release halfway through a transition: these are visible pixels, not the
  // original geometry or the target transform. New layout positions follow.
  for(const [id,visible,layout,width] of [["a",100,182,80],["b",90,36,140],["c",268,268,100]] as const){
    f.measurements.get(id)!.mockReturnValueOnce(new DOMRect(visible,0,width,26)).mockImplementation(()=>{
      expect(f.rail.dataset.reordering).toBeUndefined();
      expect(f.offset(id)).toBe("0px");
      return new DOMRect(layout,0,width,26);
    });
  }
  await f.event(window,"pointerup",124);
  expect(f.order()).toEqual(["home","b","a","c"]);
  expect(f.animations.get("a")!.animate).toHaveBeenCalledWith([
    {transform:"translateX(-82px)"},{transform:"translateX(0px)"},
  ],{duration:180,easing:"cubic-bezier(.2,.8,.2,1)"});
  expect(f.animations.get("b")!.animate).toHaveBeenCalledWith([
    {transform:"translateX(54px)"},{transform:"translateX(0px)"},
  ],{duration:180,easing:"cubic-bezier(.2,.8,.2,1)"});
  expect(f.animations.get("c")!.animate).not.toHaveBeenCalled();
  expect(f.animations.get("home")!.animate).not.toHaveBeenCalled();
  expect(f.rail.dataset.dropCommit).toBeUndefined();
  expect(f.host.querySelector("[data-drop]")).toBeNull();
  // Starting another gesture cannot retain a previous landing transform.
  f.measurements.get("a")!.mockReturnValue(new DOMRect(182,0,80,26));
  f.measurements.get("b")!.mockReturnValue(new DOMRect(36,0,140,26));
  f.measurements.get("c")!.mockReturnValue(new DOMRect(268,0,100,26));
  await f.event(f.button("a"),"pointerdown",200);
  expect(f.animations.get("a")!.cancel).toHaveBeenCalledOnce();
  expect(f.animations.get("b")!.cancel).toHaveBeenCalledOnce();
});

it("does not animate a settled drop or a reduced-motion drop",async()=>{
  const f=await setup();await f.event(f.button("a"),"pointerdown",60);await f.event(window,"pointermove",124);
  f.measurements.get("a")!.mockReturnValue(new DOMRect(182,0,80,26));
  f.measurements.get("b")!.mockReturnValue(new DOMRect(36,0,140,26));
  await f.event(window,"pointerup",124);
  expect([...f.animations.values()].every(animation=>animation.animate.mock.calls.length===0)).toBe(true);
  f.rail.style.setProperty("--reorder-motion-duration","0ms");
  await f.event(f.button("a"),"pointerdown",200);await f.event(window,"pointermove",100);await f.event(window,"pointerup",100);
  expect(f.order()).toEqual(["home","a","b","c"]);
  expect([...f.animations.values()].every(animation=>animation.animate.mock.calls.length===0)).toBe(true);
});
