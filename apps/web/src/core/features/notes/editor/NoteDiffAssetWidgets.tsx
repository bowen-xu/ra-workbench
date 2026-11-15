import { createPortal } from "react-dom";
import type { NoteBlockV1, NoteAssetContentV1 } from "@research-assistant/core/notes";
import { ManagedAssetPreview } from "./noteEditorSchema";
import type { NoteEditorDiffChunk } from "./noteDiffDecorations";
import type {ImageAlignment} from "../../../components/block-editor/ImageBlockPresentation";

export interface NoteDiffAssetTarget {
  readonly element: HTMLElement;
  readonly asset: NoteAssetContentV1;
  readonly isImage: boolean;
  readonly alignment: ImageAlignment;
  readonly previewWidth?: number | undefined;
  readonly previewScale?: number | undefined;
}

/** The upstream HTML snapshot is synchronous. Keep asynchronous managed assets
 * mounted with their existing production renderer and the original provider. */
export function noteDiffAssetTargets(widget: HTMLElement, chunk: NoteEditorDiffChunk): NoteDiffAssetTarget[] {
  const assets = (blocks: readonly NoteBlockV1[]): NoteBlockV1[] => blocks.flatMap(block => [
    ...(block.content.kind === "asset" ? [block] : []), ...assets(block.children),
  ]);
  const blocks = assets(chunk.before);
  return Array.from(widget.querySelectorAll<HTMLElement>(".managed-note-asset")).flatMap((figure, index) => {
    const block = blocks[index];
    if (!block || block.content.kind !== "asset" || figure.dataset.assetId !== block.content.assetId) return [];
    const element = widget.ownerDocument.createElement("div");
    element.className = "note-diff-asset-renderer";
    element.contentEditable = "false";
    figure.replaceWith(element);
    const alignment = block.props.textAlignment;
    return [{ element, asset: block.content, isImage: block.type === "image",
      alignment: alignment === "center" || alignment === "right" ? alignment : "left",
      previewScale: typeof block.props.previewScale === "number" ? block.props.previewScale : undefined,
      previewWidth: typeof block.props.previewWidth === "number" && block.props.previewWidth > 0 ? block.props.previewWidth : undefined }];
  });
}

export function NoteDiffAssetWidgets({ targets }: { readonly targets: readonly NoteDiffAssetTarget[] }): React.JSX.Element {
  return <>{targets.filter(target => target.element.isConnected).map(({ element, asset, isImage, alignment, previewWidth, previewScale }) => createPortal(
    <ManagedAssetPreview {...asset} isImage={isImage} alignment={alignment} previewWidth={previewWidth} previewScale={previewScale}/>, element,
  ))}</>;
}
