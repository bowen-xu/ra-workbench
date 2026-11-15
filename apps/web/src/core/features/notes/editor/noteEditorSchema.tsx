import { useEffect, useRef, useState } from "react";
import { FileText } from "lucide-react";
import { COLORS_DEFAULT } from "@blocknote/core";
import { InlineReference } from "../../../components/InlineReference";
import { createReactBlockSpec, createReactInlineContentSpec } from "@blocknote/react";
import { baseBlockEditorSchema } from "../../../components/block-editor/baseSchema";

import { useNotesClient } from "../NotesClient";
import { PageLinkPreview } from "./PageLinkPreview";
import { ReferenceLinkPreview } from "./ReferenceLinkPreview";
import {ImageBlockPresentation, type ImageAlignment} from "../../../components/block-editor/ImageBlockPresentation";
import type {ImageResizeSource} from "../../../components/block-editor/ImageBlockResize";
import {documentTitleBlockSpec} from "../../../components/block-editor/documentTitle";
import type {MediaBlockEditor} from "../../../components/block-editor/mediaBlockFocus";

const pageLink = createReactBlockSpec({ type: "pageLink", propSchema: {
  pageEntry: {default:false},
  targetKind: { default: "note_page" }, targetId: { default: "" }, labelSnapshot: { default: "笔记" }, quoteSnapshot: { default: "" },
}, content: "none" }, { render: ({ block }) => <PageLinkPreview targetId={block.props.targetId} labelSnapshot={block.props.labelSnapshot} /> });

export function ManagedAssetPreview({ assetId, displayName, mimeType, caption, isImage, alignment, onAlignmentChange, previewWidth, previewScale, resize, captionSource }: {
  readonly assetId: string;
  readonly displayName: string;
  readonly mimeType: string;
  readonly caption: string;
  readonly isImage: boolean;
  readonly alignment?: ImageAlignment | undefined;
  readonly onAlignmentChange?: ((alignment: ImageAlignment) => void) | undefined;
  readonly previewWidth?: number | undefined;
  readonly previewScale?: number | undefined;
  readonly resize?: ImageResizeSource | undefined;
  readonly captionSource?: {readonly blockId: string; readonly editor: MediaBlockEditor} | undefined;
}): React.JSX.Element {
  const client = useNotesClient();
  const [loaded, setLoaded] = useState<{
    readonly assetId: string; readonly client: typeof client; readonly mimeType: string; readonly isImage: boolean;
    readonly url?: string; readonly error?: string; readonly decodeFailed?: () => void;
  }>();
  // A prop/owner change must hide the previous bytes in the same render,
  // before the passive effect releases their URL or begins the new read.
  const current = loaded?.assetId === assetId && loaded.client === client && loaded.mimeType === mimeType && loaded.isImage === isImage ? loaded : undefined;
  useEffect(() => {
    let active = true;
    let objectUrl: string | undefined;
    const source = {assetId, client, mimeType, isImage};
    const release = () => {
      if (objectUrl !== undefined) { URL.revokeObjectURL(objectUrl); objectUrl = undefined; }
    };
    void (async () => {
      try {
        const bytes = await client.gateway.readNoteAsset(assetId);
        if (!active) return;
        objectUrl = URL.createObjectURL(new Blob([Uint8Array.from(bytes).buffer as ArrayBuffer], {type: mimeType}));
        setLoaded({...source, url: objectUrl, decodeFailed: () => {
          if (!active || objectUrl === undefined) return;
          release();
          setLoaded({...source, error: "note_asset_decode_failed：图片内容无法解码，请检查图片格式或重新插入。"});
        }});
      } catch (reason) {
        if (!active) return;
        const code = reason && typeof reason === "object" && "code" in reason && typeof reason.code === "string" ? reason.code : undefined;
        const message = reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string" ? reason.message : String(reason);
        setLoaded({...source, error: `note_asset_read_failed：${code ? `${code}：` : ""}${message}`});
      }
    })();
    return () => {
      active = false;
      release();
    };
  }, [assetId, client, mimeType, isImage]);
  if (isImage) return <ImageBlockPresentation className="managed-note-asset managedImage" assetId={assetId}
    src={current?.url} alt={caption || "图片"} caption={caption} alignment={alignment ?? "left"}
    error={current?.error} onError={current?.decodeFailed} onAlignmentChange={onAlignmentChange}
    previewWidth={previewWidth} previewScale={previewScale} resize={resize} captionSource={captionSource}/>;
  return <figure className="managed-note-asset managedFile" contentEditable={false} data-asset-id={assetId}>
    <span aria-hidden="true">▤</span>
    <figcaption><strong>{displayName}</strong><small role={current?.error ? "alert" : undefined}>{current?.error ?? (caption || mimeType)}</small></figcaption>
    {current?.url !== undefined && <a download={displayName} href={current.url}>下载</a>}
  </figure>;
}

const childPage = createReactBlockSpec(
  {
    type: "childPage",
    propSchema: {
      targetId: { default: "" },
      labelSnapshot: { default: "子页面" },
    },
    content: "none",
  },
  {
    render: ({ block }) => <PageLinkPreview targetId={block.props.targetId} labelSnapshot={block.props.labelSnapshot} />,
  },
);

const unsupportedBlock = createReactBlockSpec(
  {
    type: "unsupportedBlock",
    propSchema: {
      originalType: { default: "unknown" },
      rawJson: { default: "{}" },
    },
    content: "none",
  },
  {
    render: ({ block }) => (
      <article className="unsupported-note-block" contentEditable={false}>
        <strong>暂不支持的笔记块：{block.props.originalType}</strong>
        <details><summary>查看保留的原始数据</summary><pre>{block.props.rawJson}</pre></details>
      </article>
    ),
  },
);

function createManagedAssetBlock(type: "managedImage" | "managedFile") {
  return createReactBlockSpec(
    {
      type,
      propSchema: {
        assetId: { default: "" },
        displayName: { default: type === "managedImage" ? "图片" : "附件" },
        mimeType: { default: "application/octet-stream" },
        caption: { default: "" },
        textAlignment: {default: "left", values: ["left", "center", "right"]},
        previewScale: {default: 0}, previewWidth: {default: 0},
      },
      content: "none",
    },
    {
      render: ({ block, editor }) => <ManagedAssetPreview
        assetId={block.props.assetId}
        caption={block.props.caption}
        displayName={block.props.displayName}
        isImage={type === "managedImage"}
        mimeType={block.props.mimeType}
        alignment={block.props.textAlignment}
        previewWidth={block.props.previewWidth || undefined}
        previewScale={block.props.previewScale || undefined}
        captionSource={{blockId: block.id, editor}}
        resize={{blockId: block.id, editor, onScaleChange: previewScale => {
          if (editor.isEditable && editor.getBlock(block.id)) editor.updateBlock(block.id, {props: {previewScale, previewWidth: 0}});
        }}}
        onAlignmentChange={textAlignment => { if (editor.isEditable) editor.updateBlock(block.id, {props: {textAlignment}}); }}
      />,
    },
  );
}

const researchMentionBase = createReactInlineContentSpec(
  {
    type: "researchMention",
    propSchema: {
      targetKind: {
        default: "paper",
        values: ["note_page", "reading_note", "paper", "claim", "manuscript"],
      },
      targetId: { default: "" },
      labelSnapshot: { default: "研究对象" },
    },
    content: "styled",
  },
  {
    render: function ResearchMention({ inlineContent, contentRef }) {
      const pointer = useRef<{ x: number; y: number } | null>(null);
      const colors = new Set(inlineContent.content.map(part => part.styles.textColor ?? "default"));
      const color = colors.size === 1 ? [...colors][0] : undefined;
      // Decorations on the label must use the stored text color after reload,
      // even when the editor's color mark is nested inside the node view.
      const labelColor = color && color !== "default"
        ? COLORS_DEFAULT[color as keyof typeof COLORS_DEFAULT]?.text ?? color : undefined;
      const open = () => {
        const { targetKind, targetId, labelSnapshot } = inlineContent.props;
        if (!targetId) return;
        if (targetKind === "note_page") window.dispatchEvent(new CustomEvent("research-assistant:open-note-page", { detail: targetId }));
        else if (targetKind === "reading_note") window.dispatchEvent(new CustomEvent("research-assistant:open-reading-note", { detail: targetId }));
        else window.dispatchEvent(new CustomEvent("research-assistant:open-research-target", { detail: { kind: targetKind, id: targetId, labelSnapshot } }));
      };
      return <InlineReference className="research-mention" data-target-id={inlineContent.props.targetId}
        style={{ color: labelColor }}
        role="link" tabIndex={0} contentEditable={false} aria-disabled={!inlineContent.props.targetId}
        onPointerDown={event => { pointer.current = { x: event.clientX, y: event.clientY }; }}
        onClick={event => {
          const start = pointer.current;
          if (start && Math.hypot(event.clientX - start.x, event.clientY - start.y) > 6) return;
          open();
        }} onKeyDown={event => { if (event.key === "Enter") { event.preventDefault(); event.stopPropagation(); open(); } }}
        label={inlineContent.props.labelSnapshot} icon={<FileText aria-hidden="true" />}
        truncate={false} tone="inherit" underline="dashed" labelRef={contentRef} />;
    },
  },
);

// Atom selection keeps the stored styled label, while ProseMirror treats the
// reference as one object instead of allowing a caret inside its title.
const researchMention = { ...researchMentionBase, implementation: { ...researchMentionBase.implementation,
  node: researchMentionBase.implementation.node.extend({ atom: true }),
} };

export const noteEditorSchema = baseBlockEditorSchema.extend({
  blockSpecs: {
    documentTitle: documentTitleBlockSpec,
    pageLink: pageLink(),
    childPage: childPage(),
    referenceLink: createReactBlockSpec({ type: "referenceLink", propSchema: {
      pageEntry: {default:false},
      targetKind: { default: "paper", values: ["note_page", "reading_note", "paper", "claim", "manuscript"] },
      targetId: { default: "" }, labelSnapshot: { default: "研究对象" }, quoteSnapshot: { default: "" },
      referenceType: { default: "researchObjectReference", values: ["researchObjectReference", "readingNoteReference"] },
    }, content: "none" }, { render: ({ block }) => <ReferenceLinkPreview
      targetKind={block.props.targetKind} targetId={block.props.targetId} labelSnapshot={block.props.labelSnapshot}
    /> })(),
    unsupportedBlock: unsupportedBlock(),
    managedImage: createManagedAssetBlock("managedImage")(),
    managedFile: createManagedAssetBlock("managedFile")(),
  },
  inlineContentSpecs: {
    researchMention,
  },
});

export type NoteEditorInstance = typeof noteEditorSchema.BlockNoteEditor;
export type NoteEditorBlock = typeof noteEditorSchema.Block;
export type PartialNoteEditorBlock = typeof noteEditorSchema.PartialBlock;
