import { Menu } from "./overlay/OverlayPopup";
import "./ActionMenu.css";
import { MenuButton, MenuItem, MenuProvider, useMenuContext, useMenuStore } from "@ariakit/react";
import type { ComponentProps, ReactElement, ReactNode, Ref } from "react";
import { ControlHint } from "./ControlHint";
export { useMenuStore as useActionMenuStore } from "@ariakit/react";

/** Input-triggered actions keep the caret in the editor while Ariakit owns the popup. */
export function InputActionMenu({ store, label, className, children }: {
  readonly store: ReturnType<typeof useMenuStore>;
  readonly label: string;
  readonly className: string;
  readonly children: ReactNode;
}): React.JSX.Element {
  return <MenuProvider store={store}>
    <Menu className={className} aria-label={label} gutter={6} fixed portal modal={false}
      autoFocusOnShow={false} autoFocusOnHide={false} focusOnMove={false}>
      {children}
    </Menu>
  </MenuProvider>;
}

interface ActionMenuProps {
  readonly label: string;
  readonly menuLabel?: string;
  readonly trigger: ReactNode;
  readonly children: ReactNode;
  readonly className: string;
  readonly triggerClassName?: string;
  readonly contentClassName?: string;
  readonly disabled?: boolean;
  readonly hint?: boolean;
  readonly showTitle?: boolean;
  readonly triggerRef?: Ref<HTMLButtonElement>;
  readonly triggerRender?: ReactElement;
  readonly portalElement?: ComponentProps<typeof Menu>["portalElement"];
  readonly open?: boolean;
  readonly setOpen?: (open: boolean) => void;
  readonly autoFocusOnHide?: NonNullable<ComponentProps<typeof Menu>["autoFocusOnHide"]>;
  readonly hideOnEscape?: NonNullable<ComponentProps<typeof Menu>["hideOnEscape"]>;
  readonly placement?: NonNullable<ComponentProps<typeof MenuProvider>>["placement"];
  readonly portal?: boolean;
  /** Stateful pickers and in-flight configuration work keep their owner alive. */
  readonly keepMounted?: boolean;
}

/** Ariakit owns pointer/keyboard dismissal, navigation, placement and focus. */
export function ActionMenu(props: ActionMenuProps): React.JSX.Element {
  const parentMenu = useMenuContext();
  const trigger = <MenuButton {...(props.triggerRender ? { render: props.triggerRender } : {})} className={props.triggerClassName ?? "action-menu-trigger"} ref={props.triggerRef}
    disabled={props.disabled ?? false} aria-label={props.label}
    {...(props.open === undefined ? {} : {"aria-expanded": props.open})}>{props.trigger}</MenuButton>;
  return <MenuProvider placement={props.placement ?? (parentMenu ? "right-start" : "bottom-start")} {...(props.open === undefined ? {} : { open: props.open })} {...(props.setOpen === undefined ? {} : { setOpen: props.setOpen })}>
    <div className={props.className}>
      {props.hint || props.showTitle !== false ? <ControlHint content={props.label}>{trigger}</ControlHint> : trigger}
      <Menu className={`action-menu ${props.contentClassName ?? ""}`} aria-label={props.menuLabel ?? props.label}
        unmountOnHide={!props.keepMounted} gutter={6} portal={props.portal ?? false} {...(props.portalElement === undefined ? {} : { portalElement: props.portalElement })} fixed modal={false} autoFocusOnHide={props.autoFocusOnHide ?? true}
        {...(props.hideOnEscape === undefined ? {} : {hideOnEscape: props.hideOnEscape})}>
        {props.children}
      </Menu>
    </div>
  </MenuProvider>;
}

/** The same action may also appear outside a menu, e.g. on the project list. */
export function ActionMenuItem({hideOnClick = true, ...props}: ComponentProps<"button"> & {readonly hideOnClick?: boolean}): React.JSX.Element {
  const store = useMenuContext();
  return store
    ? <MenuItem store={store} hideOnClick={hideOnClick} disabled={props.disabled ?? false} render={<button type="button" {...props} />} />
    : <button type="button" {...props} />;
}
