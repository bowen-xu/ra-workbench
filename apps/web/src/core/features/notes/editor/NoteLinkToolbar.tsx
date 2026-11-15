import { DeleteLinkButton, EditLinkButton, LinkToolbar, useComponentsContext, type LinkToolbarProps } from "@blocknote/react";
import { Link2 } from "lucide-react";
import { isNoteSourceHref, navigateNoteSource } from "./noteSourceLinks";
import { NoteLinkEditingContext } from "./noteLinkComponents";
import { BrowserLinkToolbar } from "../../../components/block-editor/BrowserLinkToolbar";

/** Keep upstream editing controls, with explicit resource/browser navigation. */
export function NoteLinkToolbar(props: LinkToolbarProps): React.JSX.Element {
  const components = useComponentsContext()!;
  if (!isNoteSourceHref(props.url)) return <NoteLinkEditingContext.Provider value={props}><BrowserLinkToolbar {...props} /></NoteLinkEditingContext.Provider>;
  const label = "打开来源";
  return <NoteLinkEditingContext.Provider value={props}><LinkToolbar {...props}>
    <EditLinkButton {...props} />
    <components.LinkToolbar.Button className="bn-button" mainTooltip={label} label={label} isSelected={false}
      onClick={() => navigateNoteSource(props.url)} icon={<Link2 />} />
    <DeleteLinkButton range={props.range} {...(props.setToolbarOpen ? { setToolbarOpen: props.setToolbarOpen } : {})} />
  </LinkToolbar></NoteLinkEditingContext.Provider>;
}
