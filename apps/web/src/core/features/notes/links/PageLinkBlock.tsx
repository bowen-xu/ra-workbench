import type {PageLinkBlockProps} from "@research-assistant/workbench-sdk/note-presentation";
import { FileText } from "lucide-react";
import { InlineReference } from "../../../components/InlineReference";
import "./PageLinkBlock.css";

/** Shared title-only page block, used by documents and the favorites index. */
export function PageLinkBlock({ title, targetId, onOpen, onPin, onKeyDown, disabled = false, unavailable = false, appearance = "reference", navigationLabel }: PageLinkBlockProps): React.JSX.Element {
  return <InlineReference as="button" role="link" type="button" className={`note-page-link-block${unavailable ? " unavailable" : ""}`} contentEditable={false}
    data-target-id={targetId} disabled={disabled} aria-label={navigationLabel ?? `打开笔记：${title || "无标题"}`} onClick={onOpen} onDoubleClick={onPin} onKeyDown={onKeyDown}
    data-page-link-appearance={appearance} label={title || "无标题"} labelContent={appearance === "plain" ? title || "无标题" : <strong>{title || "无标题"}</strong>}
    icon={<FileText aria-hidden="true" />} tone="inherit" underline={appearance === "plain" ? "none" : "dashed"} />;
}
