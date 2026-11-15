import {useLayoutEffect,useRef,type RefObject} from "react";
import {createWheelGestureIntent} from "./wheelGestureIntent";
import "./nestedScroll.css";

const region="[data-nested-scroll-region]";
const gestureQuietMs=250;
function canScroll(node:HTMLElement,direction:number):boolean {
 return direction>0?node.scrollHeight-node.clientHeight-node.scrollTop>1:node.scrollTop>1;
}
function ownerAtStart(node:HTMLElement,root:HTMLElement,direction:number):HTMLElement|null {
 let current=node;
 while(true){
  if(canScroll(current,direction))return current;
  if(current===root)return null;
  const parent=current.parentElement?.closest<HTMLElement>(region);
  current=parent&&root.contains(parent)?parent:root;
 }
}
/** Register viewports; gesture ownership belongs to their common root. */
export function useNestedScrollRegion(ref:RefObject<HTMLElement|null>,enabled=true):void {
 useLayoutEffect(()=>{
  const node=ref.current;if(!node||!enabled)return;
  node.dataset.nestedScrollRegion="true";
  return()=>{delete node.dataset.nestedScrollRegion;};
 },[ref,enabled]);
}
/** Keep momentum on its owner; a new active gesture at its boundary may hand
 * outward immediately. The quiet timer only restores idle layout/inward entry.
 * Inactive viewports also reject native scrolling: some momentum events cannot
 * be canceled, and skipping those events must not let a child steal the gesture. */
export function useNestedWheelScroll(ref:RefObject<HTMLElement|null>,enabled=true,onScrollIntent?:(owner:HTMLElement,deltaY:number)=>void):void {
 const intent=useRef(onScrollIntent);
 useLayoutEffect(()=>{intent.current=onScrollIntent;},[onScrollIntent]);
 useLayoutEffect(()=>{
  if(!ref.current||!enabled)return;
  const root=ref.current;
  let owner:HTMLElement|null=null,idleTimer:number|undefined;
  let retainedOwner:HTMLElement|null=null;
  const intentDetector=createWheelGestureIntent();
  const blocked=new Set<HTMLElement>();
  function release():void {
   window.clearTimeout(idleTimer);idleTimer=undefined;
   if(owner){retainedOwner=owner;delete owner.dataset.nestedScrollOwner;}
   for(const node of blocked)delete node.dataset.nestedScrollBlocked;
   blocked.clear();owner=null;
  }
  function keepGesture():void {
   window.clearTimeout(idleTimer);
   idleTimer=window.setTimeout(release,gestureQuietMs);
  }
  function blockOtherViewports():void {
   for(const node of [root,...root.querySelectorAll<HTMLElement>(region)]){
    if(node!==owner&&!blocked.has(node)){node.dataset.nestedScrollBlocked="true";blocked.add(node);}
   }
  }
  function scroll(event:Event):void {
   if(owner&&event.target===owner)keepGesture();
  }
  function wheel(event:WheelEvent):void {
   if(event.ctrlKey||event.metaKey)return;
   if(event.deltaX===0&&event.deltaY===0){
    intentDetector.feed(event);
    if(owner)keepGesture();
    return;
   }
   if(Math.abs(event.deltaX)>=Math.abs(event.deltaY)||event.deltaY===0)return;
   const {momentum:coasting,restarted}=intentDetector.feed(event);
   if(owner&&!root.contains(owner)){release();retainedOwner=null;}
   // Only hand outward from the current owner, never to a child that moved
   // under the pointer. The new active packet is applied to the parent once.
   if(owner&&owner!==root&&restarted&&!coasting&&!canScroll(owner,Math.sign(event.deltaY))){
    const parent=owner.parentElement?.closest<HTMLElement>(region);
    const next=ownerAtStart(parent&&root.contains(parent)?parent:root,root,Math.sign(event.deltaY));
    if(next){release();owner=next;}
   }
   if(!owner){
    const target=event.target instanceof Element?event.target:undefined;
    const inside=target?.closest<HTMLElement>(region);
    owner=coasting&&retainedOwner&&root.contains(retainedOwner)?retainedOwner:ownerAtStart(inside&&root.contains(inside)?inside:root,root,Math.sign(event.deltaY));
   }
   if(owner){owner.dataset.nestedScrollOwner="true";blockOtherViewports();intent.current?.(owner,event.deltaY);keepGesture();}
   event.stopPropagation();
   // Noncancelable packets use native scrolling, constrained to the same owner.
   // Applying their delta as well would scroll twice.
   if(!event.cancelable)return;
   event.preventDefault();
   if(!owner)return;
   const scale=event.deltaMode===1?parseFloat(getComputedStyle(owner).lineHeight)||16:event.deltaMode===2?owner.clientHeight:1;
   owner.scrollTop+=event.deltaY*scale;
  }
  root.addEventListener("wheel",wheel,{passive:false,capture:true});
  root.addEventListener("scroll",scroll,true);
  return()=>{intentDetector.dispose();release();retainedOwner=null;root.removeEventListener("wheel",wheel,true);root.removeEventListener("scroll",scroll,true);};
 },[ref,enabled]);
}
