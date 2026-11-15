import { createContext, forwardRef, useContext, useRef, type ReactNode } from "react";
import { FormattingToolbarExtension, LinkToolbarExtension } from "@blocknote/core/extensions";
import { useBlockNoteEditor, useExtension, type ComponentProps, type LinkToolbarProps } from "@blocknote/react";
import { isAllowedNoteHref } from "@research-assistant/core/notes";
import { blockEditorComponents } from "../../../components/block-editor/blockEditorComponents";
import { notify } from "../../../services/notifications";
import { isNoteSourceHref } from "./noteSourceLinks";

export const NoteLinkEditingContext = createContext<LinkToolbarProps | undefined>(undefined);
const FormValues = createContext<{ current: Record<string, string> } | undefined>(undefined);
const needsHrefHandling = (href: string) => isNoteSourceHref(href)
  || /^[a-z][a-z\d+.-]*:/iu.test(href) && !isAllowedNoteHref(href);
function NoteForm({ children }: { readonly children?: ReactNode }) {
  const values = useRef<Record<string, string>>({});
  const Root = blockEditorComponents.Generic.Form.Root;
  return <FormValues.Provider value={values}><Root>{children}</Root></FormValues.Provider>;
}

/** Keep upstream forms and controls. Its URL submission otherwise prefixes every
 * unlisted scheme with https://, even when links.isValidLink explicitly allows it.
 */
const NoteLinkInput = forwardRef<HTMLInputElement, ComponentProps["Generic"]["Form"]["TextInput"]>(function NoteLinkInput(props, ref) {
  const values = useContext(FormValues);
  const editing = useContext(NoteLinkEditingContext);
  const editor = useBlockNoteEditor();
  const links = useExtension(LinkToolbarExtension);
  const formatting = useExtension(FormattingToolbarExtension);
  const Input = blockEditorComponents.Generic.Form.TextInput;
  if (values && props.name) values.current[props.name] = String(props.value ?? "");
  const submitSource = () => {
    const raw = values?.current.url;
    if (!raw || !needsHrefHandling(raw)) return false;
    try {
      if (!editor.isEditable) return true;
      if (!isAllowedNoteHref(raw)) throw new Error("链接协议不安全或不受支持");
      links.editLink(raw, values.current.title ?? editor.getSelectedText(), editing?.range.from ?? editor.prosemirrorState.selection.from);
      editing?.setToolbarOpen?.(false);
      editing?.setToolbarPositionFrozen?.(false);
      formatting.store.setState(false);
    } catch (reason) { notify.error(reason instanceof Error ? reason.message : "无法修改链接"); }
    return true;
  };
  return <Input {...props} ref={ref}
    onChange={event => {
      if (values && props.name) values.current[props.name] = event.currentTarget.value;
      props.onChange?.(event);
    }}
    onKeyDown={event => {
      if (event.key === "Enter" && !event.nativeEvent.isComposing && needsHrefHandling(values?.current.url ?? "")) {
        event.preventDefault(); event.stopPropagation(); submitSource();
      } else props.onKeyDown?.(event);
    }}
    onSubmit={() => { if (!submitSource()) props.onSubmit?.(); }}
  />;
});

export const noteEditorComponents = { ...blockEditorComponents, Generic: { ...blockEditorComponents.Generic,
  Form: { ...blockEditorComponents.Generic.Form, Root: NoteForm, TextInput: NoteLinkInput },
} };
