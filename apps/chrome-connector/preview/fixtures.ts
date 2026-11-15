// SPDX-License-Identifier: AGPL-3.0-or-later
namespace ConnectorPreview {
  export const papers = [
    { id: "paper-1", title: "Evidence accumulation in adaptive learning systems", authors: "Lin Chen · Maya Patel", year: "2026" },
    { id: "paper-2", title: "A practical framework for reproducible literature discovery", authors: "Alex Morgan · Lin Chen", year: "2025" },
    { id: "paper-3", title: "Connecting scientific documents through structured evidence", authors: "Maya Patel · Sam Rivera", year: "2026" },
  ] as const;
  export const scenarios = ["success", "downloading", "failed", "offline", "conflict", "incompatible", "replay"] as const;
  export type Scenario = typeof scenarios[number];
  export function scenario(value: string | null): Scenario {
    return scenarios.find(entry => entry === value) ?? "success";
  }
  export function selectedPapers(value: string | null) {
    const ids = value?.split(",") ?? [papers[0].id];
    return papers.filter(paper => ids.includes(paper.id));
  }
}
