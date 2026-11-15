import { ToolbarItem } from "@ariakit/react";
import { components } from "@blocknote/ariakit";
import { isSafari } from "@blocknote/core";
import type { ComponentProps } from "@blocknote/react";
import { forwardRef, type ComponentType, type RefAttributes } from "react";
import { ControlHint } from "../ControlHint";

/** Keep Ariakit's toolbar focus/trigger contracts, with the application's hint. */
const HintToolbarButton = forwardRef<HTMLButtonElement, ComponentProps["Generic"]["Toolbar"]["Button"]>(
  function HintToolbarButton({ className, children, mainTooltip, secondaryTooltip, icon, isSelected,
    isDisabled, onClick, label, variant: _variant, ...rest }, ref) {
    const button = <ToolbarItem ref={ref} className={`bn-ak-button bn-ak-secondary${className ? ` ${className}` : ""}`}
      aria-label={label} aria-pressed={isSelected} data-selected={isSelected ? "true" : undefined}
      disabled={isDisabled ?? false} onClick={onClick}
      onMouseDown={event => { if (isSafari()) event.currentTarget.focus(); }} {...rest}>
      {icon}{children}
    </ToolbarItem>;
    return mainTooltip ? <ControlHint content={mainTooltip} shortcut={secondaryTooltip}>{button}</ControlHint> : button;
  },
);

/** Keep the native menu/drag button; only its row/column grip artwork changes. */
// The generic registry omits DOM refs; the pinned Ariakit root forwards one.
const NativeTableHandleRoot = components.TableHandle.Root as ComponentType<ComponentProps["TableHandle"]["Root"] & RefAttributes<HTMLButtonElement>>;
const TableGripRoot = forwardRef<HTMLButtonElement, ComponentProps["TableHandle"]["Root"]>(
  function TableGripRoot({children, ...props}, ref) {
    const grip = props.className?.split(/\s+/u).includes("bn-table-handle");
    return <NativeTableHandleRoot {...props} ref={ref}>
      {grip ? <svg className="note-table-handle-dots" viewBox="0 0 8 15" aria-hidden="true" focusable="false" fill="currentColor">
        <circle cx="2" cy="2.5" r="1.15"/><circle cx="6" cy="2.5" r="1.15"/>
        <circle cx="2" cy="7.5" r="1.15"/><circle cx="6" cy="7.5" r="1.15"/>
        <circle cx="2" cy="12.5" r="1.15"/><circle cx="6" cy="12.5" r="1.15"/>
      </svg> : children}
    </NativeTableHandleRoot>;
  },
);

// Use the public component registry rather than changing third-party styles or
// replacing editor commands. Other Ariakit components remain upstream.
export const blockEditorComponents = {
  ...components,
  FormattingToolbar: { ...components.FormattingToolbar, Button: HintToolbarButton },
  LinkToolbar: { ...components.LinkToolbar, Button: HintToolbarButton },
  TableHandle: { ...components.TableHandle, Root: TableGripRoot },
  Generic: { ...components.Generic, Toolbar: { ...components.Generic.Toolbar, Button: HintToolbarButton } },
};
