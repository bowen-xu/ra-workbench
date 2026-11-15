import {useRef, useState} from "react";
import {BlockCaption} from "./BlockCaption";
import {useEditorReadOnly} from "./EditorReadOnly";
import type {ImageAlignment} from "./ImageBlockIcons";
import {ImageBlockResize, type ImageResizeSource} from "./ImageBlockResize";
import {BlockMediaControls} from "./BlockMediaControls";
import {focusMediaBlock, type MediaBlockEditor} from "./mediaBlockFocus";
import {isImageScale, useImageAvailableWidth} from "./imageSizing";
import {focusBlockCaption} from "./captionBlock";
import "./ImageBlockPresentation.css";

export type {ImageAlignment} from "./ImageBlockIcons";

/** Image content and controls are shared; the host owns resources and history. */
export function ImageBlockPresentation({src, alt, caption, alignment = "left", error, onError, onAlignmentChange, className = "", assetId, previewWidth, previewScale, resize, captionSource}: {
  readonly src?: string | undefined;
  readonly alt: string;
  readonly caption: string;
  readonly alignment?: ImageAlignment;
  readonly error?: string | undefined;
  readonly onError?: (() => void) | undefined;
  readonly onAlignmentChange?: ((alignment: ImageAlignment) => void) | undefined;
  readonly className?: string;
  readonly assetId?: string;
  readonly previewWidth?: number | undefined;
  readonly previewScale?: number | undefined;
  readonly resize?: ImageResizeSource | undefined;
  readonly captionSource?: {readonly blockId: string; readonly editor: MediaBlockEditor} | undefined;
}): React.JSX.Element {
  const readOnly = useEditorReadOnly();
  const captionButton = useRef<HTMLButtonElement>(null);
  const figure = useRef<HTMLElement>(null);
  const availableWidth = useImageAvailableWidth(figure, resize?.editor);
  const [dragScale, setDragScale] = useState<number>();
  const scale = isImageScale(previewScale) ? previewScale : undefined;
  const effectiveScale = dragScale ?? scale;
  const width = effectiveScale && availableWidth ? availableWidth * effectiveScale / 100 : previewWidth;
  const canEdit = !readOnly && (!!captionSource || !!onAlignmentChange);
  const canResize = !readOnly && resize && src;
  const media = <div className="note-image-block-media block-media-surface" style={!canResize && width ? {width} : undefined}>
        {src ? <img src={src} alt={alt} onError={onError} />
          : <span className="note-image-block-placeholder" role="status">{error ? "图片无法显示" : "正在加载图片…"}</span>}
        {canEdit && <BlockMediaControls label="图片" alignment={alignment} onAlignmentChange={onAlignmentChange}
          onCaptionEdit={captionSource ? () => focusBlockCaption(captionSource.editor, captionSource.blockId) : undefined} captionButtonRef={captionButton}
          onReturnFocus={captionSource ? () => {focusMediaBlock(captionSource.editor, captionSource.blockId);} : undefined}/>}
      </div>;
  return <figure ref={figure} className={`note-image-block ${className}`} contentEditable={false} data-asset-id={assetId}
      data-alignment={alignment} data-preview-width={!!width} data-scale={effectiveScale}>
      {canResize ? <ImageBlockResize key={assetId} source={resize} width={width} availableWidth={availableWidth}
        scale={scale} alignment={alignment} onPreview={setDragScale}>{media}</ImageBlockResize> : media}
      {error && <small className="note-image-block-error" role="alert">{error}</small>}
      {!captionSource && caption && <figcaption><BlockCaption value={caption} label="图片说明"/></figcaption>}
    </figure>;
}
