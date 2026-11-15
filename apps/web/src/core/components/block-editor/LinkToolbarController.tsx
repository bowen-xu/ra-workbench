import {useLayoutEffect, useState, type ComponentProps} from "react";
import {LinkToolbarController as NativeLinkToolbarController, useBlockNoteEditor} from "@blocknote/react";
import {LinkToolbarExtension} from "@blocknote/core/extensions";
import {linkHoverRange} from "./linkHoverRange";

/** One adapter for every host. Retain the native controller, hover timing,
 * selection lookup and commands; replace only its DOM hit lookup through the
 * public extension lifecycle, without mutating the upstream instance. */
export function LinkToolbarController(props: ComponentProps<typeof NativeLinkToolbarController>): React.JSX.Element | null {
  const editor = useBlockNoteEditor();
  const [installed, setInstalled] = useState<typeof editor>();
  useLayoutEffect(() => {
    const original = editor.getExtension(LinkToolbarExtension);
    if (!original) throw new Error("link_toolbar_extension_missing：链接工具栏扩展未注册。");
    const adapted = {...original, getLinkAtElement: (element: HTMLElement) => linkHoverRange(editor.prosemirrorView, element)};
    editor.replaceExtension(original, () => adapted);
    setInstalled(editor);
    return () => {
      if (editor.getExtension(LinkToolbarExtension) === adapted) editor.replaceExtension(adapted, () => original);
    };
  }, [editor]);
  // The controller must subscribe to the installed extension, including on a
  // retained-host editor switch or React StrictMode remount.
  return installed === editor ? <NativeLinkToolbarController {...props}/> : null;
}
