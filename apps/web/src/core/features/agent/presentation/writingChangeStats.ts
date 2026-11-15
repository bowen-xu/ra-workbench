import { diff } from "@codemirror/merge";

export interface WritingWorkspaceChangeStats { readonly additions: number; readonly deletions: number }

/** Count whole changed lines, not contextual lines in editor merge chunks.
 * Interning lines lets the existing diff algorithm compare line identities. */
export function writingLineDiff(before: string, after: string) {
  const identities = new Map<string, string>();
  function lines(text: string): string[] { if(!text)return [];const values=text.split("\n");if(text.endsWith("\n"))values.pop();return values; }
  function encode(text: string): string {
    return lines(text).map(line => {
      let token = identities.get(line);
      if (token === undefined) {
        // Start above the surrogate range; CodeMirror respects code point
        // boundaries when comparing strings. Reviewed text is capped at 1 MiB.
        token = String.fromCodePoint(0xe000 + identities.size);
        identities.set(line, token);
      }
      return token;
    }).join("");
  }
  const a = encode(before), b = encode(after);
  function positions(text:string):ReadonlyMap<number,number>{const offsets=new Map([[0,0]]);let offset=0,index=0;for(const point of text){offset+=point.length;offsets.set(offset,++index);}return offsets;}
  const aPositions=positions(a),bPositions=positions(b);
  // CodeMirror compares at code-point boundaries, which are all indexed above.
  return {before:lines(before),after:lines(after),changes:diff(a,b,{scanLimit:1000,timeout:50}).map(change=>({fromA:aPositions.get(change.fromA)!,toA:aPositions.get(change.toA)!,fromB:bPositions.get(change.fromB)!,toB:bPositions.get(change.toB)!}))};
}
/** Shared line identities keep cards and frozen tool previews consistent. */
export function countWritingChangedLines(before: string, after: string): WritingWorkspaceChangeStats {
  return writingLineDiff(before,after).changes.reduce((total, change) => ({
    additions: total.additions + change.toB-change.fromB,
    deletions: total.deletions + change.toA-change.fromA,
  }), { additions: 0, deletions: 0 });
}
