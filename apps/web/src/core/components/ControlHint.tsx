import { Tooltip } from "./overlay/OverlayPopup";
import { TooltipAnchor, TooltipProvider, useTooltipStore } from "@ariakit/react";
import type { MouseEvent, ReactElement, ReactNode } from "react";

import "./ControlHint.css";

/** Renders into the existing control so hints do not change its layout or behavior. */
export function ControlHint({ children, content, shortcut, wrap = false, preferSidePlacement = false, tooltipClassName }: {
  readonly children: ReactElement;
  readonly content: ReactNode;
  readonly shortcut?: ReactNode;
  readonly wrap?: boolean;
  readonly preferSidePlacement?: boolean;
  readonly tooltipClassName?: string;
}): React.JSX.Element {
  const tooltip = useTooltipStore({ placement: preferSidePlacement ? "left" : "top", timeout: 300, hideTimeout: 0 });
  const hasContent = content !== undefined && content !== null && content !== "" && content !== false;
  const hideOnMouseLeave = (event: MouseEvent<HTMLElement>) => {
    // Hover hints belong to the control, not Ariakit's transit polygon.
    if (!event.currentTarget.hasAttribute("data-focus-visible")) tooltip.hide();
  };
  return <TooltipProvider store={tooltip}>
    {wrap ? <TooltipAnchor showOnHover={hasContent} onFocusVisible={event => { if (!hasContent) event.preventDefault(); }} onMouseLeave={hideOnMouseLeave} render={<span className="control-hint-anchor" />}>{children}</TooltipAnchor> : <TooltipAnchor showOnHover={hasContent} onFocusVisible={event => { if (!hasContent) event.preventDefault(); }} onMouseLeave={hideOnMouseLeave} render={children} />}
    {hasContent && <Tooltip unmountOnHide portal gutter={6} flip={preferSidePlacement ? "right bottom top" : true} overlap={false} disablePointerEventsOnApproach={false} className={`control-hint${tooltipClassName ? ` ${tooltipClassName}` : ""}`}>
      <span className="control-hint-content">{content}</span>
      {shortcut && <kbd>{shortcut}</kbd>}
    </Tooltip>}
  </TooltipProvider>;
}
