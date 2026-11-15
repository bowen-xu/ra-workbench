import type { ReactNode } from "react";
import { ArrowLeft, ArrowRight, PanelLeftClose, PanelLeftOpen } from "lucide-react";
import { isDesktopRuntime } from "../runtime/platformCore";
import { ControlHint } from "./ControlHint";
import { NotificationRestoreButton } from "./NotificationRestoreButton";
import "./AppWindowToolbar.css";

export function AppWindowToolbar({ canGoBack, canGoForward, onBack, onForward, sidebarExpanded, onToggleSidebar, onTabHostChange, tabHostActive = false, tabs }: {
  readonly canGoBack: boolean;
  readonly canGoForward: boolean;
  readonly onBack: () => void;
  readonly onForward: () => void;
  readonly sidebarExpanded: boolean;
  readonly onToggleSidebar: () => void;
  readonly onTabHostChange?: (element: HTMLDivElement | null) => void;
  readonly tabHostActive?: boolean;
  readonly tabs?: ReactNode;
}): React.JSX.Element {
  const nativeMac = isDesktopRuntime() && /Mac/.test(navigator.platform);
  const sidebarLabel = sidebarExpanded ? "收起主导航" : "展开主导航";
  // Tauri's deep region includes blank descendants but excludes interactive controls.
  // Its native handler owns dragging and macOS mouseup double-click maximization.
  return <header className="app-window-toolbar" data-tauri-drag-region={isDesktopRuntime() ? "deep" : undefined} data-native-controls={nativeMac || undefined} data-sidebar-expanded={sidebarExpanded} aria-label="窗口工具栏">
    <div className="app-window-navigation" role="toolbar" aria-label="工作区导航">
      <ControlHint content="后退"><button aria-label="后退" disabled={!canGoBack} onClick={onBack} type="button"><ArrowLeft aria-hidden="true" /></button></ControlHint>
      <ControlHint content="前进"><button aria-label="前进" disabled={!canGoForward} onClick={onForward} type="button"><ArrowRight aria-hidden="true" /></button></ControlHint>
      <ControlHint content={sidebarLabel}><button aria-label={sidebarLabel} aria-controls="app-sidebar" aria-expanded={sidebarExpanded} onClick={onToggleSidebar} type="button">
        {sidebarExpanded ? <PanelLeftClose aria-hidden="true" /> : <PanelLeftOpen aria-hidden="true" />}
      </button></ControlHint>
    </div>
    <div className="app-window-tab-host" hidden={!tabHostActive} ref={onTabHostChange}>{tabs}</div>
    <div className="app-window-drag-region" aria-hidden="true" />
    <NotificationRestoreButton />
  </header>;
}
