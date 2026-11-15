import { Menu as AriakitMenu, Popover as AriakitPopover, SelectPopover as AriakitSelectPopover, Tooltip as AriakitTooltip,
  type MenuProps, type PopoverProps, type SelectPopoverProps, type TooltipProps } from "@ariakit/react";
import { overlayStyle, useOverlayPortal } from "./overlayScope";
import "./overlay.css";

/** Closing a child popup must not also trigger the browser's native dialog cancel. */
function nativeDialogEscape(setting: MenuProps["hideOnEscape"]): NonNullable<MenuProps["hideOnEscape"]> {
  return event => {
    const accepted = typeof setting === "function" ? setting(event) : setting ?? true;
    if (accepted && event.target instanceof Element && event.target.closest("dialog[open]")) event.preventDefault();
    return accepted;
  };
}

export function Menu(props: MenuProps): React.JSX.Element {
  const portalElement = useOverlayPortal("popup", props.portalElement);
  return <AriakitMenu {...props} hideOnEscape={nativeDialogEscape(props.hideOnEscape)} fixed portal portalElement={portalElement} style={overlayStyle("popup", props.style)} />;
}
export function Popover(props: PopoverProps): React.JSX.Element {
  const portalElement = useOverlayPortal("popup", props.portalElement);
  return <AriakitPopover {...props} hideOnEscape={nativeDialogEscape(props.hideOnEscape)} fixed portal portalElement={portalElement} style={overlayStyle("popup", props.style)} />;
}
export function SelectPopover(props: SelectPopoverProps): React.JSX.Element {
  const portalElement = useOverlayPortal("popup", props.portalElement);
  return <AriakitSelectPopover {...props} hideOnEscape={nativeDialogEscape(props.hideOnEscape)} fixed portal portalElement={portalElement} style={overlayStyle("popup", props.style)} />;
}
export function Tooltip(props: TooltipProps): React.JSX.Element {
  const portalElement = useOverlayPortal("hint", props.portalElement);
  return <AriakitTooltip {...props} fixed portal portalElement={portalElement} style={overlayStyle("hint", props.style)} />;
}
