import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { Maximize2, X } from "lucide-react";
import { ControlHint } from "../ControlHint";
import { OverlayPortal } from "../overlay/OverlayPortal";
import { OverlayScopeContext, useOverlayPanelScope } from "../overlay/overlayScope";
import { useNoteWindowGeometry } from "./useNoteWindowGeometry";
import "./NoteFloatingWindow.css";

/** Storage/editor differences belong to the injected session, never the window UI. */
export interface NoteWindowSession {
  readonly prepareClose: () => Promise<boolean>;
  readonly openTab: (accepted: () => void) => Promise<boolean | void>;
  readonly dismiss: () => void;
  readonly reportError: (reason: unknown) => void;
  readonly beginOperation?: () => boolean;
  readonly endOperation?: () => void;
}

export function NoteFloatingWindow({ open = true, identity, title, label, closeLabel, resizeLabel,
  session, panelRef, canExpand = true, className = "", children }: {
  readonly open?: boolean; readonly identity: string; readonly title: string; readonly label: string;
  readonly closeLabel: string; readonly resizeLabel: string; readonly session: NoteWindowSession;
  readonly panelRef?: RefObject<HTMLElement | null>; readonly canExpand?: boolean; readonly className?: string;
  readonly children: ReactNode | ((state: {readonly busy: boolean}) => ReactNode);
}): React.JSX.Element | null {
  const [panel, setPanel] = useState<HTMLElement | null>(null), [busy, setBusy] = useState(false);
  const scope = useOverlayPanelScope(panel), geometry = useNoteWindowGeometry();
  const pending = useRef(false), alive = useRef(true), opener = useRef<HTMLElement | null>(null);
  const latest = useRef({open, identity}); latest.current = {open, identity};
  const attachPanel = useCallback((element: HTMLElement | null) => { setPanel(element); if (panelRef) panelRef.current = element; }, [panelRef]);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  useLayoutEffect(() => {
    if (open) opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  }, [open, identity]);
  async function act(expand: boolean): Promise<void> {
    if (pending.current || !open || expand && !canExpand || session.beginOperation?.() === false) return;
    pending.current = true; setBusy(true);
    let accepted = false;
    const owns = () => alive.current && latest.current.open && latest.current.identity === identity;
    const finish = () => {
      if (accepted || !owns()) return;
      accepted = true; session.dismiss();
      if (!expand && opener.current?.isConnected) opener.current.focus({preventScroll: true});
    };
    try {
      if (!await session.prepareClose() || !owns()) return;
      if (expand && await session.openTab(finish) === false) return;
      finish();
    } catch (reason) { if (owns()) session.reportError(reason); }
    finally { session.endOperation?.(); pending.current = false; if (alive.current) setBusy(false); }
  }
  if (!open) return null;
  return <OverlayPortal kind="surface"><OverlayScopeContext value={scope}>
    <section className={`note-floating-window reading-note-floating-panel ${className}`} role="dialog" aria-modal={false}
      aria-label={label} aria-busy={busy} tabIndex={-1} style={geometry.style} ref={attachPanel}
      onKeyDownCapture={event => {
        if (event.key !== "Escape" || event.defaultPrevented || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229
          || event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return;
        const target = event.target instanceof Element ? event.target : null;
        const layer = target?.closest('[role="menu"], [role="listbox"], [role="dialog"]');
        if (layer && layer !== panel || target?.closest('[aria-expanded="true"][aria-controls]')) return;
        event.preventDefault(); event.stopPropagation(); void act(false);
      }}>
      <header onPointerDown={geometry.startDrag}><strong>{title}</strong><div className="reading-note-window-actions">
        <ControlHint content="在笔记标签页中打开"><button type="button" aria-label="在笔记标签页中打开" disabled={busy || !canExpand} onClick={() => void act(true)}><Maximize2 size={16} /></button></ControlHint>
        <ControlHint content={closeLabel}><button type="button" aria-label={closeLabel} disabled={busy} onClick={() => void act(false)}><X size={16} /></button></ControlHint>
      </div></header>
      <div className="note-floating-window-body" inert={busy}>{typeof children === "function" ? children({busy}) : children}</div>
      {(["nw", "ne", "sw", "se"] as const).map(corner => <div key={corner} className={`reading-note-resize ${corner}`} role="separator"
        aria-label={`调整${resizeLabel} ${corner} 大小`} onPointerDown={event => geometry.startResize(corner, event)} />)}
    </section>
  </OverlayScopeContext></OverlayPortal>;
}
