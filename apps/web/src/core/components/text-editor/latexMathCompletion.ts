import { pickedCompletion, snippetCompletion, type Completion, type CompletionSource } from "@codemirror/autocomplete";

const argumentsByCommand: Readonly<Record<string, number>> = {
  frac: 2, dfrac: 2, tfrac: 2, binom: 2, sqrt: 1, text: 1, mathrm: 1, mathbf: 1,
  mathit: 1, mathbb: 1, mathcal: 1, operatorname: 1, overline: 1, underline: 1,
  hat: 1, widehat: 1, bar: 1, vec: 1, dot: 1, ddot: 1, overbrace: 1, underbrace: 1,
};

/** Shared mathematical vocabulary; argument fields use CodeMirror snippets. */
const argumentCommands: readonly Completion[] = Object.entries(argumentsByCommand).map(([label, count]) => {
  const completion = snippetCompletion(
    label + Array.from({ length: count }, (_, index) => `{\${${index + 1}}}`).join("") + "${0}",
    { label, displayLabel: `\\${label}`, type: "function" },
  );
  return { ...completion, apply(view, chosen, from, to) {
    if (["{", "["].includes(view.state.sliceDoc(to, to + 1))) {
      view.dispatch({ changes: { from, to, insert: label }, selection: { anchor: from + label.length + 1 },
        annotations: pickedCompletion.of(chosen), userEvent: "input.complete", scrollIntoView: true });
    } else if (typeof completion.apply === "function") completion.apply(view, chosen, from, to);
  } };
});

export const latexMathCommandOptions: readonly Completion[] = [
  ...argumentCommands,
  ...("alpha beta gamma delta epsilon varepsilon zeta eta theta vartheta iota kappa lambda mu nu xi pi varpi rho varrho sigma varsigma tau upsilon phi varphi chi psi omega "
    + "Gamma Delta Theta Lambda Xi Pi Sigma Upsilon Phi Psi Omega "
    + "sum prod int iint iiint oint lim sin cos tan cot sec csc arcsin arccos arctan sinh cosh tanh log ln exp min max sup inf det gcd "
    + "infty partial nabla pm mp times div cdot circ ast leq geq neq approx equiv sim propto in notin subset supset subseteq supseteq cup cap emptyset forall exists neg land lor "
    + "to mapsto rightarrow leftarrow leftrightarrow Rightarrow Leftarrow Leftrightarrow ldots cdots vdots ddots lbrace rbrace langle rangle left right big Big bigg Bigg")
    .split(" ").map(label => ({ label, displayLabel: `\\${label}`, type: "keyword" })),
];

export const latexMathCompletionSource: CompletionSource = context => {
  const match = context.matchBefore(/\\[A-Za-z]*$/u);
  if (!match) return null;
  let slashes = 1;
  for (let index = match.from - 1; index >= 0 && context.state.sliceDoc(index, index + 1) === "\\"; index--) slashes++;
  if (slashes % 2 === 0) return null; // A TeX line break is not a command prefix.
  return { from: match.from + 1, options: latexMathCommandOptions, validFor: /^[A-Za-z]*$/u };
};
