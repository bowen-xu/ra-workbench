import { File, FileCodeCorner, Folder, FolderPen, Highlighter, Image, Link2, Newspaper, StickyNote } from "lucide-react";
import type { ComponentPropsWithoutRef } from "react";
import { InlineReference } from "../../components/InlineReference";
import { ControlHint } from "../../components/ControlHint";
import { notify } from "../../services/notifications";
import type { SourceObject } from "./sourceDirectory";
import { navigateSource } from "./sourceNavigation";
import { SourceHintContent, useSourceLinkInfo } from "./SourceHintContent";
import "./AgentComposerTextInput.css";
import "./AgentSourceLink.css";

function sourceIcon(object: SourceObject) {
  switch (object.kind) {
    case "paper": return Newspaper;
    case "pdf": return File;
    case "note": return StickyNote;
    case "highlight": return Highlighter;
    case "manuscript": return FolderPen;
    case "folder": return Folder;
    case "manuscriptFile": return object.file.kind === "image" ? Image : FileCodeCorner;
    case "structuredObject": return FileCodeCorner;
    case "structuredCatalog": return Folder;
    case "directory": return Folder;
    case "systemDocument": return FileCodeCorner;
    case "root": return Link2;
  }
}

/** Icons reflect the resolved object identity; labels never determine resource types. */
export function AgentSourceLink({ path, children, ...props }: ComponentPropsWithoutRef<"a"> & { readonly path: string }) {
  const {object, lines} = useSourceLinkInfo(path);
  const Icon = object ? sourceIcon(object) : Link2;
  const hint = <SourceHintContent path={path} lines={lines}/>;
  return <ControlHint content={hint}><InlineReference as="a" {...props} label={typeof children === "string" ? children : path} labelContent={children}
    icon={<Icon aria-hidden="true" />} className="agent-composer-reference agent-source-reference" onClick={event => {
    event.preventDefault();
    void navigateSource(path).catch(reason => notify.error(reason instanceof Error ? reason.message : "无法打开来源"));
  }} /></ControlHint>;
}
