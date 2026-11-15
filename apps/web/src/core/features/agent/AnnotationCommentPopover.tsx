import { Popover } from "../../components/overlay/OverlayPopup";
import { usePopoverStore } from "@ariakit/react";
import { AnnotationCommentEditor } from "./AnnotationCommentEditor";
import type { SourceNavigationLocation } from "./sourceNavigation";
import { useEffect, useRef } from "react";

export function AnnotationCommentPopover({location, label = "编辑来源批注", ...props}: React.ComponentProps<typeof AnnotationCommentEditor> & {readonly location: SourceNavigationLocation; readonly label?: string}) {
  const popover = usePopoverStore({open: true, placement: "bottom-start", setOpen: open => {if (!open) props.onCancel();}});
  const reposition = useRef<(() => Promise<void>) | undefined>(undefined);
  const cancel = useRef(props.onCancel); cancel.current = props.onCancel;
  useEffect(() => {
    // Virtual text/region anchors have no DOM ancestor for automatic scroll observation.
    const update = () => {if (!location.getAnchorRect()) {cancel.current(); return;} void reposition.current?.().catch(() => cancel.current());};
    window.addEventListener("scroll", update, true);
    return () => window.removeEventListener("scroll", update, true);
  }, [location]);
  return <Popover store={popover} portal gutter={8} getAnchorRect={() => location.getAnchorRect()} updatePosition={({updatePosition}) => {reposition.current = updatePosition; return updatePosition();}} className="annotation-comment-popover" aria-label={label} autoFocusOnHide={false}>
    <AnnotationCommentEditor {...props}/>
  </Popover>;
}
