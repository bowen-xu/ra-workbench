/** @vitest-environment happy-dom */
import {act,useRef} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {useNestedWheelScroll,useNestedScrollRegion} from "./useNestedWheelScroll";
import doubleSwipe from "./__fixtures__/double-swipe-right.json";
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
function Fixture({intent}:{intent?:((owner:HTMLElement)=>void)|undefined}){
 const chat=useRef<HTMLDivElement>(null),inner=useRef<HTMLDivElement>(null),history=useRef<HTMLDivElement>(null);
 useNestedWheelScroll(chat,true,intent);useNestedScrollRegion(inner);useNestedScrollRegion(history);
 return <div ref={chat}><div ref={history} data-testid="history"><div ref={inner} data-testid="inner"/></div></div>;
}
beforeEach(()=>vi.useFakeTimers({toFake:["setTimeout","clearTimeout"]}));
afterEach(()=>vi.useRealTimers());
async function fixture(intent?:((owner:HTMLElement)=>void)){
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 await act(async()=>root.render(<Fixture intent={intent}/>));
 const chat=host.firstElementChild as HTMLElement,history=host.querySelector<HTMLElement>('[data-testid="history"]')!,inner=host.querySelector<HTMLElement>('[data-testid="inner"]')!;
 for(const node of [chat,history,inner]){
  let top=0;Object.defineProperties(node,{scrollHeight:{value:1000},clientHeight:{value:100},scrollTop:{get:()=>top,set:(value:number)=>{top=Math.max(0,Math.min(900,value));}}});
 }
 return {chat,history,inner,rerender:async(next:((owner:HTMLElement)=>void))=>act(async()=>root.render(<Fixture intent={next}/>)),cleanup:async()=>{await act(async()=>root.unmount());host.remove();}};
}
function wheel(node:HTMLElement,delta:number,extra:WheelEventInit={}){
 const event=new WheelEvent("wheel",{deltaY:delta,bubbles:true,cancelable:true,...extra});
 for(const [key,value] of Object.entries(extra))Object.defineProperty(event,key,{value});
 node.dispatchEvent(event);return event;
}
it("holds boundary momentum, acceleration and direction noise; only new input after quiet can move outward",async()=>{
 const f=await fixture();try{
  wheel(f.inner,900);vi.advanceTimersByTime(16);wheel(f.inner,30);vi.advanceTimersByTime(16);wheel(f.inner,8);
  vi.advanceTimersByTime(16);wheel(f.inner,10);wheel(f.inner,-2);wheel(f.inner,2);
  expect(f.inner.scrollTop).toBe(900);expect(f.history.scrollTop).toBe(0);expect(f.chat.scrollTop).toBe(0);
  vi.advanceTimersByTime(251);
  expect(f.history.scrollTop).toBe(0);expect(f.chat.scrollTop).toBe(0);
  wheel(f.inner,50);expect(f.history.scrollTop).toBe(50);expect(f.chat.scrollTop).toBe(0);
  f.history.scrollTop=900;vi.advanceTimersByTime(251);wheel(f.inner,50);expect(f.chat.scrollTop).toBe(50);
 }finally{await f.cleanup();}
});
it("does not let sparse inertia or changing hit targets steal an outer gesture, and waits for actual scrolling too",async()=>{
 const f=await fixture();try{
  wheel(f.chat,60);vi.advanceTimersByTime(120);wheel(f.inner,30);vi.advanceTimersByTime(120);wheel(f.inner,16);
  expect(f.inner.scrollTop).toBe(0);expect(f.history.scrollTop).toBe(0);expect(f.chat.scrollTop).toBe(106);
  vi.advanceTimersByTime(200);f.chat.dispatchEvent(new Event("scroll"));vi.advanceTimersByTime(120);wheel(f.inner,8);
  expect(f.inner.scrollTop).toBe(0);expect(f.chat.scrollTop).toBe(114);
  vi.advanceTimersByTime(251);wheel(f.inner,30);expect(f.inner.scrollTop).toBe(30);
  wheel(f.chat,20);expect(f.inner.scrollTop).toBe(50);expect(f.chat.scrollTop).toBe(114);
 }finally{await f.cleanup();}
});
it("constrains noncancelable native packets without double-applying them, while preserving zoom and horizontal input",async()=>{
 const intent=vi.fn(),f=await fixture(intent);try{
  wheel(f.chat,40);const before=f.chat.scrollTop;
  const packet=wheel(f.inner,20,{cancelable:false});
  expect(packet.defaultPrevented).toBe(false);expect(f.chat.scrollTop).toBe(before);
  expect(f.inner.dataset.nestedScrollBlocked).toBe("true");expect(f.history.dataset.nestedScrollBlocked).toBe("true");
  expect(f.chat.dataset.nestedScrollOwner).toBe("true");expect(intent).toHaveBeenLastCalledWith(f.chat,20);
  const calls=intent.mock.calls.length;
  expect(wheel(f.inner,10,{ctrlKey:true}).defaultPrevented).toBe(false);
  expect(wheel(f.inner,10,{metaKey:true}).defaultPrevented).toBe(false);
  expect(wheel(f.inner,10,{deltaX:40}).defaultPrevented).toBe(false);expect(intent).toHaveBeenCalledTimes(calls);
  vi.advanceTimersByTime(251);expect(f.inner.dataset.nestedScrollBlocked).toBeUndefined();expect(f.chat.dataset.nestedScrollOwner).toBeUndefined();
 }finally{await f.cleanup();}
});
it("keeps line/page wheel input on its owner and removes every lock on unmount",async()=>{
 const f=await fixture();f.chat.style.lineHeight="20px";
 wheel(f.chat,2,{deltaMode:1});wheel(f.inner,1,{deltaMode:2});expect(f.chat.scrollTop).toBe(140);expect(f.inner.scrollTop).toBe(0);
 await f.cleanup();expect(f.inner.dataset.nestedScrollBlocked).toBeUndefined();expect(f.chat.dataset.nestedScrollOwner).toBeUndefined();
});
it("does not reset a gesture when its callback changes during a render",async()=>{
 const first=vi.fn(),next=vi.fn(),f=await fixture(first);try{
  wheel(f.chat,20);await f.rerender(next);wheel(f.inner,20);
  expect(f.inner.scrollTop).toBe(0);expect(f.chat.scrollTop).toBe(40);expect(next).toHaveBeenCalledWith(f.chat,20);
 }finally{await f.cleanup();}
});
it("hands a new active gesture outward immediately while native momentum is still running",async()=>{
 const f=await fixture();try{
  wheel(f.inner,900,{momentum:false} as WheelEventInit);vi.advanceTimersByTime(16);
  wheel(f.inner,30,{momentum:true} as WheelEventInit);vi.advanceTimersByTime(16);
  wheel(f.inner,80,{momentum:true} as WheelEventInit);expect(f.history.scrollTop).toBe(0);
  vi.advanceTimersByTime(16);wheel(f.inner,2,{momentum:false} as WheelEventInit);
  expect(f.history.scrollTop).toBe(2);expect(f.inner.scrollTop).toBe(900);
  f.history.scrollTop=900;vi.advanceTimersByTime(16);wheel(f.inner,1,{momentum:true} as WheelEventInit);
  vi.advanceTimersByTime(16);wheel(f.inner,3,{momentum:false} as WheelEventInit);
  expect(f.chat.scrollTop).toBe(3);
 }finally{await f.cleanup();}
});
it("hands the first packet of the second recorded trackpad swipe outward without a quiet delay",async()=>{
 const f=await fixture();let time=doubleSwipe.wheelEvents[0]!.timeStamp;
 try{
  for(const [index,packet] of doubleSwipe.wheelEvents.entries()){
   vi.advanceTimersByTime(packet.timeStamp-time);time=packet.timeStamp;
   wheel(f.inner,-packet.deltaX,{deltaX:packet.deltaY,timeStamp:packet.timeStamp} as WheelEventInit);
   if(index===32){expect(f.inner.scrollTop).toBe(900);expect(f.history.scrollTop).toBe(0);}
   if(index===33){expect(f.history.scrollTop).toBe(9);break;}
  }
 }finally{await f.cleanup();}
});
it("retains sparse native momentum after idle cleanup and preserves a zero-delta active restart",async()=>{
 const f=await fixture();try{
  wheel(f.inner,900,{momentum:false} as WheelEventInit);
  wheel(f.inner,8,{momentum:true} as WheelEventInit);vi.advanceTimersByTime(300);
  wheel(f.inner,2,{momentum:true} as WheelEventInit);expect(f.history.scrollTop).toBe(0);
  wheel(f.inner,0,{momentum:false} as WheelEventInit);
  wheel(f.inner,1,{momentum:false} as WheelEventInit);expect(f.history.scrollTop).toBe(1);
 }finally{await f.cleanup();}
});
it("does not turn a late equal-size fallback momentum packet into active input after detector timeout",async()=>{
 const f=await fixture();let time=doubleSwipe.wheelEvents[0]!.timeStamp;
 try{
  for(const packet of doubleSwipe.wheelEvents.slice(0,33)){
   vi.advanceTimersByTime(packet.timeStamp-time);time=packet.timeStamp;
   wheel(f.inner,-packet.deltaX,{deltaX:packet.deltaY,timeStamp:time} as WheelEventInit);
  }
  vi.advanceTimersByTime(300);wheel(f.inner,2,{timeStamp:time+300} as WheelEventInit);
  expect(f.inner.scrollTop).toBe(900);expect(f.history.scrollTop).toBe(0);
 }finally{await f.cleanup();}
});
