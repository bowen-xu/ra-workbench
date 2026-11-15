import { DeleteLinkButton, EditLinkButton, LinkToolbar, useComponentsContext, type LinkToolbarProps } from "@blocknote/react";
import { ExternalLink } from "lucide-react";
import { isWebUrl } from "../../runtime/browserAdapter";
import { openBrowserPageFromUi } from "../../runtime/browserLinkRouting";

/** Reuse BlockNote editing controls while replacing its direct window.open. */
export function BrowserLinkToolbar(props: LinkToolbarProps): React.JSX.Element {
  const components = useComponentsContext()!;
  if (!isWebUrl(props.url)) return <LinkToolbar {...props} />;
  const label = "在浏览器中打开";
  return <LinkToolbar {...props}>
    <EditLinkButton {...props} />
    <components.LinkToolbar.Button className="bn-button" mainTooltip={label} label={label} isSelected={false}
      onClick={() => openBrowserPageFromUi(props.url)} icon={<ExternalLink />} />
    <DeleteLinkButton range={props.range} {...(props.setToolbarOpen ? {setToolbarOpen: props.setToolbarOpen} : {})} />
  </LinkToolbar>;
}
