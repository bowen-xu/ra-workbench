import {useEffect, useLayoutEffect, useRef, useState, type RefObject} from "react";
import {Check, ChevronRight, Ellipsis} from "lucide-react";
import {PopoverDisclosure, PopoverProvider, usePopoverStore, useStoreState} from "@ariakit/react";
import {Popover} from "../overlay/OverlayPopup";
import {ControlHint} from "../ControlHint";
import {useEditorReadOnly} from "./EditorReadOnly";
import {ImageAlignmentIcon, ImageCaptionIcon, type ImageAlignment} from "./ImageBlockIcons";
import {NoteActionMenu, NoteActionMenuItem} from "./NoteActionMenu";
import "./BlockMediaControls.css";

const alignments = [
  {value: "left", label: "左对齐"},
  {value: "center", label: "居中对齐"},
  {value: "right", label: "右对齐"},
] as const;

/** Shared controls keep selection and resource mutations in the owning block. */
export function BlockMediaControls({label, alignment, onAlignmentChange, onCaptionEdit, captionButtonRef, onReturnFocus}: {
  readonly label: "图片" | "表格";
  readonly alignment: ImageAlignment;
  readonly onAlignmentChange?: ((alignment: ImageAlignment) => void) | undefined;
  readonly onCaptionEdit?: (() => void) | undefined;
  readonly captionButtonRef?: RefObject<HTMLButtonElement | null>;
  readonly onReturnFocus?: (() => void) | undefined;
}): React.JSX.Element | null {
  const readOnly = useEditorReadOnly();
  const alignmentButton = useRef<HTMLButtonElement>(null);
  const menuButton = useRef<HTMLButtonElement>(null), controls = useRef<HTMLDivElement>(null);
  const [compact, setCompact] = useState(false), [menuOpen, setMenuOpen] = useState(false), [alignMenuOpen, setAlignMenuOpen] = useState(false);
  const popover = usePopoverStore({placement: "bottom-end"});
  const open = useStoreState(popover, "open");
  const returnFocus = () => {
    if (onReturnFocus) onReturnFocus();
    else (compact ? menuButton.current : alignmentButton.current)?.focus({preventScroll: true});
  };
  useEffect(() => {if (readOnly) {popover.hide(); setMenuOpen(false); setAlignMenuOpen(false);}}, [readOnly, popover]);
  useLayoutEffect(() => {
    const media = controls.current?.closest<HTMLElement>(".block-media-surface");
    if (!media) return;
    const measure = () => {
      const width = media.getBoundingClientRect().width;
      // Two 28px icons plus frame/insets need room independent of zoom scale.
      setCompact(width > 0 && width < 96);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure); observer.observe(media);
    return () => observer.disconnect();
  }, [readOnly, !!onAlignmentChange, !!onCaptionEdit]);
  useEffect(() => {popover.hide(); setMenuOpen(false); setAlignMenuOpen(false);}, [compact, popover]);
  if (readOnly || !onAlignmentChange && !onCaptionEdit) return null;
  return <PopoverProvider store={popover}>
    <div ref={controls} className="block-media-controls" role="toolbar" aria-label={`${label}操作`}
      data-compact={compact} data-active={open || menuOpen}>
      {compact ? <NoteActionMenu label={`${label}操作菜单`} menuLabel={`${label}操作`} className="block-media-menu"
        trigger={<Ellipsis aria-hidden="true"/>} triggerClassName="block-media-menu-trigger" showTitle={false} hint portal placement="bottom-end"
        triggerRef={element => {menuButton.current = element; if (captionButtonRef) captionButtonRef.current = element;}}
        autoFocusOnHide={false} hideOnEscape={() => {returnFocus(); return true;}}
        open={menuOpen} setOpen={value => {setMenuOpen(value); if (!value) setAlignMenuOpen(false);}}>
        {onAlignmentChange && <NoteActionMenu label={`${label}对齐方式`} menuLabel={`选择${label}对齐方式`}
          className="block-media-alignment-menu" triggerClassName="note-action-menu-item" showTitle={false} portal
          open={alignMenuOpen} setOpen={setAlignMenuOpen} autoFocusOnHide={() => menuOpen}
          trigger={<><ImageAlignmentIcon alignment={alignment}/><span>对齐方式</span><ChevronRight aria-hidden="true"/></>}>
          {alignments.map(({value, label: alignmentLabel}) => <NoteActionMenuItem key={value} role="menuitemradio"
            aria-checked={alignment === value} onClick={() => {
              if (readOnly) return;
              onAlignmentChange(value); setAlignMenuOpen(false); setMenuOpen(false); returnFocus();
            }}><ImageAlignmentIcon alignment={value}/><span>{alignmentLabel}</span>{alignment === value && <Check aria-hidden="true"/>}</NoteActionMenuItem>)}
        </NoteActionMenu>}
        {onCaptionEdit && <NoteActionMenuItem onClick={() => {if (!readOnly) {setMenuOpen(false); onCaptionEdit();}}}>
          <ImageCaptionIcon/><span>编辑说明</span>
        </NoteActionMenuItem>}
      </NoteActionMenu> : <>
      {onAlignmentChange && <ControlHint content={`${label}对齐方式`}><PopoverDisclosure store={popover} ref={alignmentButton}
        type="button" aria-label={`${label}对齐方式`} onMouseDown={event => event.preventDefault()}
        onClickCapture={event => event.currentTarget.focus({preventScroll: true})}>
        <ImageAlignmentIcon alignment={alignment}/>
      </PopoverDisclosure></ControlHint>}
      {onCaptionEdit && <ControlHint content={`编辑${label}说明`}><button type="button" ref={captionButtonRef}
        aria-label={`编辑${label}说明`} onMouseDown={event => event.preventDefault()}
        onClick={() => {if (!readOnly) {popover.hide(); onCaptionEdit();}}}>
        <ImageCaptionIcon/>
      </button></ControlHint>}
      </>}
    </div>
    {!compact && onAlignmentChange && <Popover store={popover} portal fixed modal={false} unmountOnHide gutter={6}
      fitViewport overflowPadding={12} className="block-media-alignment-popup" aria-label={`选择${label}对齐方式`}
      autoFocusOnHide={false} hideOnEscape={() => {returnFocus(); return true;}}>
      <div role="group" aria-label={`${label}对齐选项`}>
        {alignments.map(({value, label: alignmentLabel}) => <ControlHint key={value} content={alignmentLabel}><button type="button"
          aria-label={alignmentLabel} aria-pressed={alignment === value} onClick={() => {
            if (readOnly) return;
            onAlignmentChange(value); popover.hide(); returnFocus();
          }}><ImageAlignmentIcon alignment={value}/></button></ControlHint>)}
      </div>
    </Popover>}
  </PopoverProvider>;
}
