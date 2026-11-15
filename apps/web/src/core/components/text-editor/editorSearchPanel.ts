import "./EditorSearchPanel.css";
import { closeSearchPanel, findNext, findPrevious, getSearchQuery, replaceAll, replaceNext, SearchQuery, setSearchQuery } from "@codemirror/search";
import { EditorView, type Panel } from "@codemirror/view";
import { editorSearchScope, inSearchScope, preserveReplacementCase, replacementFor, setSearchScope, type SearchRange } from "./editorSearchBehavior";
import { createSearchIcons, type SearchIcon } from "./editorSearchIcons";

const histories = new WeakMap<EditorView, { find: string[]; replace: string[] }>();
export function createEditorSearchPanel(view: EditorView): Panel {
  const icons = createSearchIcons();
  function button(label: string, run: () => unknown, shortcut?: string, icon?: SearchIcon): HTMLButtonElement {
    const control = searchButton(label, run, shortcut, icon ? icons.icon(icon) : undefined);
    icons.hint(control, shortcut);
    return control;
  }
  const dom = document.createElement("div"); dom.className = "text-search-panel";
  dom.setAttribute("role", "search"); dom.setAttribute("aria-label", "当前文件查找替换");
  const mac = /Mac|iPhone|iPad/.test(navigator.platform), mod = mac ? "⌘" : "Ctrl+", alt = mac ? "⌥⌘" : "Alt+";
  const history = histories.get(view) ?? { find: [], replace: [] }; histories.set(view, history);
  let selected = view.state.selection.ranges.filter(range => !range.empty).map(({ from, to }) => ({ from, to }));
  let historyIndex = -1, historyDraft = "";
  const search = input("查找内容", "查找（↑↓ 查看历史）"), replacement = input("替换为", "替换为");
  search.setAttribute("main-field", "true"); search.name = "search";
  const query = getSearchQuery(view.state); search.value = query.search; replacement.value = query.replace;
  const status = document.createElement("span"); status.className = "editor-search-status"; status.setAttribute("role", "status");
  const caseSensitive = toggle("区分大小写", "Aa", query.caseSensitive, `${alt}C`);
  const wholeWord = toggle("全词匹配", "ab", query.wholeWord, `${alt}W`); wholeWord.className = "search-whole-word";
  const regexp = toggle("正则表达式", ".*", query.regexp, `${alt}R`);
  const preserve = toggle("保留大小写", "AB", false, `${alt}P`);
  const scoped = toggle("在选区中查找", "", !!view.state.field(editorSearchScope, false), `${alt}L`, "selection");
  const previous = button("上一项", () => navigate(false), "Shift+Enter", "up");
  const next = button("下一项", () => navigate(true), "Enter", "down");
  const close = button("关闭查找", () => { remember(); closeSearchPanel(view); view.focus(); }, "Escape", "close");
  const replace = button("替换", () => replaceMatches(false), "Enter", "replace");
  const replaceEvery = button("全部替换", () => replaceMatches(true), `${mod}Enter`, "replaceAll");
  const replaceRow = document.createElement("div"); replaceRow.className = "editor-replace-row"; replaceRow.hidden = true;
  const expand = button("展开替换", () => {
    replaceRow.hidden = !replaceRow.hidden; expand.setAttribute("aria-expanded", String(!replaceRow.hidden));
    expand.setAttribute("aria-label", replaceRow.hidden ? "展开替换" : "收起替换");
    icons.mount();
  }, undefined, "chevron"); expand.className = "editor-search-expand"; expand.setAttribute("aria-expanded", "false");
  const field = document.createElement("div"); field.className = "text-search-field"; field.append(search, caseSensitive, wholeWord, regexp);
  const replaceField = document.createElement("div"); replaceField.className = "text-search-field"; replaceField.append(replacement, preserve);
  const searchRow = document.createElement("div"); searchRow.className = "editor-find-row"; searchRow.append(field, status, previous, next, scoped, close);
  const replaceActions = document.createElement("div"); replaceActions.className = "editor-replace-actions"; replaceActions.append(replace, replaceEvery);
  replaceRow.append(replaceField, replaceActions);
  dom.append(expand, searchRow, replaceRow); expand.disabled = view.state.readOnly;
  function remember(): void {
    for (const [value, list] of [[search.value, history.find], [replacement.value, history.replace]] as const) {
      if (value) { const index = list.indexOf(value); if (index >= 0) list.splice(index, 1); list.unshift(value); list.splice(50); }
    }
  }
  function navigate(forward: boolean): void { remember(); (forward ? findNext : findPrevious)(view); }
  function replaceMatches(all: boolean): void {
    if (view.state.readOnly || !getSearchQuery(view.state).valid || view.composing) return;
    remember();
    if (preserve.getAttribute("aria-pressed") !== "true") { (all ? replaceAll : replaceNext)(view); return; }
    const current = getSearchQuery(view.state), matches = collectMatches(current);
    const selection = view.state.selection.main;
    const ranges = all ? matches : matches.filter(range => range.from === selection.from && range.to === selection.to);
    if (!ranges.length) { if (!all) navigate(true); return; }
    const doc = view.state.doc.toString();
    const changes = ranges.map(range => {
      const expanded = replacementFor(current, doc, range);
      const insert = current.regexp && /\$(?:[1-9]|&)/.test(current.replace) ? expanded : preserveReplacementCase(expanded, doc.slice(range.from, range.to));
      return { ...range, insert };
    });
    view.dispatch({ changes, userEvent: "input.replace" });
    if (!all) navigate(true);
  }
  function commit(): void {
    view.dispatch({ effects: setSearchQuery.of(new SearchQuery({ search: search.value, replace: replacement.value, literal: true,
      caseSensitive: caseSensitive.getAttribute("aria-pressed") === "true", regexp: regexp.getAttribute("aria-pressed") === "true",
      wholeWord: wholeWord.getAttribute("aria-pressed") === "true", test: (match, state, from, to) => inSearchScope(match, state, from, to) })) });
  }
  function collectMatches(query: SearchQuery, limit = Infinity): SearchRange[] {
    const matches: SearchRange[] = [], cursor = query.getCursor(view.state);
    for (let match = cursor.next(); !match.done && matches.length < limit; match = cursor.next()) matches.push(match.value);
    return matches;
  }
  function refresh(): void {
    const current = getSearchQuery(view.state); search.value = current.search;
    const matches = current.valid ? collectMatches(current, 10001) : [];
    const selection = view.state.selection.main, index = matches.findIndex(range => range.from === selection.from && range.to === selection.to);
    status.textContent = current.search && !current.valid ? "正则表达式无效" : matches.length > 10000 ? "超过 10000 个匹配" : !matches.length ? "无结果" : `${index < 0 ? "?" : index + 1} / ${matches.length}`;
    search.setAttribute("aria-invalid", String(!!current.search && !current.valid));
    next.disabled = previous.disabled = !matches.length;
    replace.disabled = replaceEvery.disabled = !matches.length || view.state.readOnly;
    scoped.disabled = !selected.length && scoped.getAttribute("aria-pressed") !== "true";
  }
  function toggle(label: string, text: string, active: boolean, shortcut: string, icon?: SearchIcon): HTMLButtonElement {
    const control = button(label, () => {
      const enable = control.getAttribute("aria-pressed") !== "true"; control.setAttribute("aria-pressed", String(enable));
      if (control === scoped) view.dispatch({ effects: setSearchScope.of(enable ? selected : null) });
      commit();
      const current = getSearchQuery(view.state), selection = view.state.selection.main;
      const matches = current.valid ? collectMatches(current, 10001) : [];
      if (matches.length && !matches.some(range => range.from === selection.from && range.to === selection.to)) findNext(view);
    }, shortcut, icon); if (!icon) control.textContent = text; control.setAttribute("aria-pressed", String(active)); return control;
  }
  search.addEventListener("input", event => { historyIndex = -1; commit(); if (!(event as InputEvent).isComposing && getSearchQuery(view.state).valid) findNext(view); }); replacement.addEventListener("input", commit);
  dom.addEventListener("keydown", event => {
    if (event.isComposing) return;
    const key = event.key.toLowerCase(), modifier = mac ? event.metaKey : true;
    if (event.altKey && modifier && ["c", "w", "r", "p", "l"].includes(key)) {
      event.preventDefault(); event.stopPropagation(); ({ c: caseSensitive, w: wholeWord, r: regexp, p: preserve, l: scoped } as Record<string, HTMLButtonElement>)[key]!.click(); return;
    }
    if (event.key === "Escape") { event.preventDefault(); close.click(); }
    else if (event.key === "Enter") { event.preventDefault();
      if (event.target === replacement && !event.shiftKey) replaceMatches(event.metaKey || event.ctrlKey);
      else navigate(!event.shiftKey);
    } else if ((event.key === "ArrowUp" || event.key === "ArrowDown") && (event.target === search || event.target === replacement)) {
      event.preventDefault(); const target = event.target as HTMLInputElement, list = target === search ? history.find : history.replace;
      if (historyIndex === -1) historyDraft = target.value;
      historyIndex = Math.max(-1, Math.min(list.length - 1, historyIndex + (event.key === "ArrowUp" ? 1 : -1)));
      target.value = historyIndex === -1 ? historyDraft : list[historyIndex]!; commit();
    }
  });
  search.addEventListener("focus", () => { historyIndex = -1; });
  replacement.addEventListener("focus", () => { historyIndex = -1; });
  refresh();
  return { dom, top: true, mount: () => queueMicrotask(() => { if (dom.isConnected) { icons.mount(); search.focus(); } }), destroy: () => { remember(); icons.destroy(); }, update(update) {
    if (update.docChanged) selected = selected.map(range => ({ from: update.changes.mapPos(range.from, -1), to: update.changes.mapPos(range.to, 1) }));
    else if (update.selectionSet && view.hasFocus) selected = view.state.selection.ranges.filter(range => !range.empty).map(({ from, to }) => ({ from, to }));
    if (update.docChanged || update.selectionSet || update.transactions.some(transaction => transaction.effects.length)) refresh();
  } };
}
function input(label: string, placeholder: string): HTMLInputElement { const element = document.createElement("input"); element.setAttribute("aria-label", label); element.placeholder = placeholder; return element; }
function searchButton(label: string, run: () => unknown, shortcut?: string, icon?: HTMLElement): HTMLButtonElement {
  const element = document.createElement("button"); element.type = "button"; element.setAttribute("aria-label", label);
  if (shortcut) element.setAttribute("aria-keyshortcuts", shortcut.replaceAll("⌥⌘", "Alt+Meta+").replaceAll("⌘", "Meta+"));
  if (icon) element.append(icon); else element.textContent = label;
  element.addEventListener("click", run); return element;
}
