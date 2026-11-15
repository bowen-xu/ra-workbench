import type { ReactNode } from "react";

export interface WorkbenchLayoutProps {
  readonly toolbar?: ReactNode;
  readonly navigation?: ReactNode;
  readonly status?: ReactNode;
  readonly utilityActions?: ReactNode;
  readonly sidebarExpanded: boolean;
  readonly writingFocused: boolean;
  /** The application supplies its document presentation provider and main slot. */
  readonly children?: ReactNode;
}

/** Production workbench frame. Domain selection and all actions stay with the
 * application; an empty frame does not discover or initialize plugins. */
export function WorkbenchLayout({toolbar, navigation, status, utilityActions, sidebarExpanded, writingFocused, children}: WorkbenchLayoutProps): React.JSX.Element {
  return <div className="app-window">
    {toolbar}
    <div className="app-shell" data-sidebar-expanded={sidebarExpanded} data-writing-focused={writingFocused}>
      <aside className="app-sidebar" id="app-sidebar" aria-label="主导航">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true" />
          <div><strong>Research</strong><span>Assistant</span></div>
        </div>
        <nav className="nav-list">{navigation}</nav>
        <div className="sidebar-footer">
          {status}
          <div className="sidebar-utility-actions">{utilityActions}</div>
        </div>
      </aside>
      {children}
    </div>
  </div>;
}
