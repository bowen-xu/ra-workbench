import { useCallback, useEffect, useRef, useState } from "react";
import {
  dictionaryWordForms, isDictionaryQuery, normalizeDictionaryQuery,
  type DictionaryGateway, type DictionaryResult, type WordExplanation, type WordExplanationRequest, type WordExplanationGateway,
} from "@research-assistant/core/dictionary";
import { notify } from "../../services/notifications";
import "./DictionaryLookup.css";
import { WordContextExplanation } from "./WordContextExplanation";

type LookupState = { readonly kind: "idle" | "loading" }
  | { readonly kind: "error" }
  | { readonly kind: "result"; readonly result: DictionaryResult };

export function DictionaryLookup({ text, gateway, explanation, explaining = false, context, explanationGateway, onDeleteExplanation, deleteDisabled = false }: {
  readonly text: string;
  readonly gateway: DictionaryGateway<AbortSignal>;
  readonly explanation?: WordExplanation;
  readonly explaining?: boolean;
  readonly context: WordExplanationRequest;
  readonly explanationGateway?: WordExplanationGateway<AbortSignal>;
  readonly onDeleteExplanation?: () => Promise<void>;
  readonly deleteDisabled?: boolean;
}): React.JSX.Element {
  const query = useRef(text);
  const [state, setState] = useState<LookupState>({ kind: "idle" });
  const request = useRef<AbortController | undefined>(undefined);

  const lookup = useCallback(async (value: string): Promise<void> => {
    query.current = value;
    request.current?.abort();
    const normalized = normalizeDictionaryQuery(value);
    if (!isDictionaryQuery(normalized)) {
      setState({ kind: "idle" });
      return;
    }
    const controller = new AbortController();
    request.current = controller;
    setState({ kind: "loading" });
    try {
      const result = await gateway.lookup(value, controller.signal);
      if (!controller.signal.aborted) setState({ kind: "result", result });
    } catch (error) {
      if (controller.signal.aborted) return;
      setState({ kind: "error" });
      notify.error(error instanceof Error ? error.message : "本地查词失败，请重试。");
    }
  }, [gateway]);

  useEffect(() => {
    void lookup(text);
    return () => request.current?.abort();
  }, [text, lookup]);

  return <section aria-label="离线词典" className="dictionary-lookup">
    {explaining && <p role="status">正在结合语境解释词义…</p>}
    {explanation && <WordContextExplanation explanation={explanation} context={context} deleteDisabled={deleteDisabled}
      {...(onDeleteExplanation ? { onDelete: onDeleteExplanation } : {})} {...(explanationGateway ? { gateway: explanationGateway } : {})} />}
    <div aria-live="polite" className="dictionary-results" aria-busy={state.kind === "loading"}>
      {state.kind === "idle" && <p>请选择一个英文单词或短语。</p>}
      {state.kind === "loading" && <p>正在查词…</p>}
      {state.kind === "error" && <button onClick={() => void lookup(query.current)} type="button">重试查词</button>}
      {state.kind === "result" && state.result.entries.length === 0 && <p>未找到“{state.result.query}”，可重新选择原形或其他拼写。</p>}
      {state.kind === "result" && (state.result.suggestions?.length ?? 0) > 0 && <div className="dictionary-suggestions" aria-label="你可能想查" data-has-match={state.result.entries.length > 0}>
        <span>你可能想查…</span><div>{state.result.suggestions?.map(suggestion => <button key={suggestion.word} type="button" onClick={() => void lookup(suggestion.word)}>
          {suggestion.word}<small>{suggestion.kind === "form" ? "形式相近" : "拼写相近"}</small>
        </button>)}</div>
      </div>}
      {state.kind === "result" && state.result.entries.map((entry, index) => <article key={`${entry.word}:${index}`}>
        <header><strong>{entry.word}</strong>{entry.phonetic && <span>/{entry.phonetic}/</span>}</header>
        {normalizeDictionaryQuery(entry.word) !== state.result.query && <small>“{state.result.query}”的词形回查结果</small>}
        {entry.translation && <p>{entry.translation.replace(/\\n/g, "\n")}</p>}
        {entry.definition && <details><summary>英文释义</summary><p>{entry.definition.replace(/\\n/g, "\n")}</p></details>}
        {dictionaryWordForms(entry.exchange).length > 0 && <div className="dictionary-word-forms">
          {dictionaryWordForms(entry.exchange).map((form, formIndex) => <button key={`${form.label}:${formIndex}`} type="button"
            onClick={() => void lookup(form.word)}>{form.label} · {form.word}</button>)}
        </div>}
      </article>)}
    </div>
  </section>;
}
