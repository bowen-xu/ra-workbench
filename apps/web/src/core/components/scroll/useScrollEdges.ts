import {useLayoutEffect,useState,type RefObject} from "react";
/** Observe both the viewport and its content so disclosure/resize updates fades. */
export function useScrollEdges(ref:RefObject<HTMLDivElement|null>,revision:unknown,enabled=true){
 const [edges,setEdges]=useState({top:false,bottom:false});
 function measure(){const node=ref.current;if(!node)return;const next={top:node.scrollTop>1,bottom:node.scrollHeight-node.clientHeight-node.scrollTop>1};setEdges(old=>old.top===next.top&&old.bottom===next.bottom?old:next);}
 useLayoutEffect(()=>{if(!enabled)return;measure();const node=ref.current,observer=typeof ResizeObserver==="undefined"?undefined:new ResizeObserver(measure);if(node){observer?.observe(node);for(const child of node.children)observer?.observe(child);}return()=>observer?.disconnect();},[ref,revision,enabled]);
 return {...edges,onScroll:measure};
}
