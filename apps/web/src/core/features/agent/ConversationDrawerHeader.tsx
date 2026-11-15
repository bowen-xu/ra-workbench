import {createContext, useContext, useState, type ReactNode} from "react";
import {createPortal} from "react-dom";

const HeaderTarget = createContext<HTMLElement | null | undefined>(undefined);

/** The drawer owns placement; the list keeps ownership of search and archive state. */
export function ConversationDrawerHeader({children, closeButton}: {
  readonly children: ReactNode;
  readonly closeButton: ReactNode;
}): React.JSX.Element {
  const [target, setTarget] = useState<HTMLDivElement | null>(null);
  return <HeaderTarget.Provider value={target}>
    <header aria-label="对话列表工具栏"><div className="conversation-drawer-header-actions" ref={setTarget}/>{closeButton}</header>
    {children}
  </HeaderTarget.Provider>;
}

export function ConversationListToolbar({children}: {readonly children: ReactNode}): React.JSX.Element | null {
  const target = useContext(HeaderTarget);
  const toolbar = <div className="conversation-list-toolbar" role="toolbar" aria-label="对话列表操作">{children}</div>;
  return target === undefined ? toolbar : target ? createPortal(toolbar, target) : null;
}
