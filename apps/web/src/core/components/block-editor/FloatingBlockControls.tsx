import {createContext, useContext, useEffect, useId, useLayoutEffect, useState, type ReactNode, type RefObject} from "react";
import {usePopoverStore} from "@ariakit/react";
import {Popover} from "../overlay/OverlayPopup";
import {BlockControls} from "./BlockControls";
import type {BlockDragDom} from "./blockDragDom";

interface GutterOwner {
  readonly root: RefObject<HTMLElement|null>;
  readonly surface: HTMLElement|null;
  readonly dom: BlockDragDom;
  readonly owner: string;
  readonly active: string|undefined;
  readonly activate: (id:string)=>void;
}
const Gutter = createContext<GutterOwner|undefined>(undefined);
export function useBlockControlsRoot():RefObject<HTMLElement|null> {
  const owner=useContext(Gutter);
  if(!owner)throw new Error("Block controls require their surface owner");
  return owner.root;
}

/** One hovered gutter per surface, including the transit space left of nested rows.
 * Menus/gestures freeze the current owner, as BlockNote's SideMenu does. */
export function BlockControlsProvider({root,dom,children}:{readonly root:RefObject<HTMLElement|null>;readonly dom:BlockDragDom;readonly children:ReactNode}):React.JSX.Element {
  const owner=useId(),[active,setActive]=useState<string>();
  const [surface,setSurface]=useState<HTMLElement|null>(null);
  useLayoutEffect(()=>setSurface(root.current),[root]);
  useEffect(()=>{
    const surface=root.current;if(!surface)return;
    const doc=surface.ownerDocument;
    const update=(event:PointerEvent)=>{
      if(surface.dataset.blockDragging==="true")return;
      const controls=[...doc.querySelectorAll<HTMLElement>("[data-block-controls-owner]")].filter(node=>node.dataset.blockControlsOwner===owner);
      if(controls.some(node=>node.querySelector('[aria-expanded="true"]')))return;
      if(!(event.target instanceof Element))return;
      const gutter=event.target.closest<HTMLElement>("[data-block-controls-owner]");
      if(gutter?.dataset.blockControlsOwner===owner){setActive(gutter.dataset.blockControlsId);return;}
      const content=event.target.closest<HTMLElement>(dom.contentSelector);
      if(content&&dom.owns(surface,event.target)){setActive(dom.contentId(surface,content));return;}
      if(!dom.owns(surface,event.target)&&!event.target.contains(surface)){setActive(undefined);return;}
      // The gap between the row and its floating controls is part of the gutter.
      const row=[...surface.querySelectorAll<HTMLElement>(dom.contentSelector)].find(node=>{
        const rect=node.getBoundingClientRect();
        return event.clientY>=rect.top&&event.clientY<=rect.bottom&&event.clientX>=rect.left-72&&event.clientX<=rect.left;
      });
      setActive(row ? dom.contentId(surface,row) : undefined);
    };
    doc.addEventListener("pointermove",update,true);
    const leave=()=>setActive(undefined);doc.defaultView?.addEventListener("blur",leave);
    return ()=>{doc.removeEventListener("pointermove",update,true);doc.defaultView?.removeEventListener("blur",leave);};
  },[dom,owner,root]);
  return <Gutter.Provider value={{root,surface,dom,owner,active,activate:setActive}}>{children}</Gutter.Provider>;
}

/** Always-mounted toolbar retains keyboard/menu ownership while visually hidden.
 * Shared Popover handles portal ownership, viewport bounds, scrolling and anchoring. */
export function FloatingBlockControls({blockId,label,children}:{readonly blockId:string;readonly label:string;readonly children:ReactNode}):React.JSX.Element {
  const context=useContext(Gutter);
  if(!context)throw new Error("Floating block controls require a surface owner");
  const {surface,dom,owner,active,activate}=context;
  const popover=usePopoverStore({open:true,placement:"left-start"});
  useLayoutEffect(()=>{
    const row=surface?.querySelector<HTMLElement>(dom.rowSelector(blockId));
    popover.setAnchorElement(row?.querySelector<HTMLElement>(dom.contentSelector)??null);
  },[blockId,dom,popover,surface]);
  // Register the nonmodal toolbar only after its editor can participate in ownership.
  if(!surface)return <></>;
  return <Popover store={popover} portal modal={false} role="toolbar" aria-label={label} gutter={4} overflowPadding={4}
    getPersistentElements={()=>[surface.closest<HTMLElement>('[role="dialog"],dialog[open]')??surface]}
    autoFocusOnShow={false} autoFocusOnHide={false} hideOnEscape={false} hideOnInteractOutside={false}
    render={<BlockControls/>} className="block-controls block-floating-controls" data-block-controls-owner={owner}
    data-block-controls-surface={surface ? dom.scope(surface)?.instance : undefined}
    data-block-controls-id={blockId} data-active={active===blockId} onFocusCapture={()=>activate(blockId)}>
    {children}
  </Popover>;
}
