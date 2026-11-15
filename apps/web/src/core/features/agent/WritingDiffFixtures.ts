export type ChangeStatus = "pending" | "kept" | "undone";

export interface DemoChange {
  readonly path: string;
  readonly kind: "modified" | "added" | "deleted";
  readonly before: string;
  readonly after: string;
  readonly beforeBytes?: readonly number[];
  readonly afterBytes?: readonly number[];
  readonly additions: number;
  readonly deletions: number;
}

export const initialChanges: readonly DemoChange[] = [
  { path: "main.tex", kind: "modified", additions: 2, deletions: 2, before: String.raw`\documentclass[10pt,article]{article}
\usepackage{lipsum}
\begin{document}
\title{Logical O-learning}
\maketitle

\summary{The summary appears on the cover page. Although it can be identical to the abstract, it does not have to be.}

\section{Research question}
We investigate how agents represent and update their goals.
The framework separates means from ends.
We consider measurement, construction, and education.
This section describes the conceptual foundations.

\lipsum[1]
\end{document}`, after: String.raw`\documentclass[10pt,article]{article}
\usepackage{lipsum}
\begin{document}
\title{Logical O-learning}
\maketitle

\summary{This paper presents \emph{Logical O-learning (LQL)}, a conceptual reframing of O-learning through Non-Axiomatic Logic (NAL). We establish a parallel correspondence between NAL's representation and propagation of goals and desires and the central mechanisms of Q-learning.}

\section{Research question}
We investigate how agents represent and update their goals.
The framework separates means from ends.
We consider measurement, construction, and education.
This section describes the conceptual foundations.

The proposed framework clarifies how value information is represented and updated.
\end{document}` },
  { path: "sections/introduction.tex", kind: "modified", additions: 2, deletions: 1, before: String.raw`\section{Introduction}
This section introduces the problem and reviews related work.`, after: String.raw`\section{Introduction}
This section introduces the problem, states the research question, and reviews related work.
We distinguish autonomy over means from autonomy over ends.` },
  { path: "figures/overview.tex", kind: "modified", additions: 5, deletions: 0, before: "", after: String.raw`\begin{figure}
  \centering
  \input{figures/overview}
  \caption{Overview of the proposed framework.}
\end{figure}` },
];

/** Data only: mixed native disk targets alongside the ordinary source editor. */
export const mixedFileChanges: readonly DemoChange[] = [initialChanges[0]!,
  { path: "sections/new.tex", kind: "added", before: "", after: "\\section{New}\r\nNew section 😀\r\n", additions: 2, deletions: 0 },
  { path: "assets/old.json", kind: "deleted", before: '{"obsolete":true}\r\n', after: "", additions: 0, deletions: 1 },
];

/** Non-text bytes exercise production disk publication and immutable history. */
export const binaryFileChanges: readonly DemoChange[] = [initialChanges[0]!,
  { path: "assets/figure.bin", kind: "modified", before: "", after: "",
    beforeBytes: [0, 255, 128, 13, 10], afterBytes: [0, 254, 129, 13, 10, 42], additions: 0, deletions: 0 },
];
