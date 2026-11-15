import type {AgentEvidenceInput} from "@research-assistant/core/agent";
import {isWritingSourcePath} from "@research-assistant/writing";
import type {SourceObject} from "./sourceDirectory";

const commentCharacters = new Intl.Segmenter(undefined, {granularity: "grapheme"});
type CommentPreviewPart = {readonly kind: "text" | "lineBreak"; readonly text: string};

/** Display only: the saved comment and editor retain the original whitespace and text. */
export function annotationCommentPreview(comment: string): string {
  return annotationCommentPreviewParts(comment).map(part => part.text).join("");
}

export function annotationCommentPreviewParts(comment: string): readonly CommentPreviewPart[] {
  const characters: CommentPreviewPart[] = Array.from(commentCharacters.segment(comment.trim())).flatMap(part =>
    /^(?:\r\n|[\r\n\u2028\u2029])$/.test(part.segment)
      ? [{kind: "text", text: " "}, {kind: "lineBreak", text: "↵"}, {kind: "text", text: " "}]
      : [{kind: "text", text: part.segment}]);
  const limited: CommentPreviewPart[] = characters.length > 80
    ? [...characters.slice(0, 79), {kind: "text", text: "…"}] : characters;
  // Combine adjacent text; literal ↵ characters remain text rather than becoming icons.
  const parts: CommentPreviewPart[] = [];
  for (const part of limited) {
    const previous = parts.at(-1);
    if (part.kind === "text" && previous?.kind === "text") parts[parts.length - 1] = {kind: "text", text: previous.text + part.text};
    else parts.push(part);
  }
  return parts;
}

/** A quote from a PDF or structured note does not make its source a text file. */
export function isTextFileAnnotation(item: AgentEvidenceInput, object?: SourceObject): boolean {
  return !item.image && object?.kind === "manuscriptFile" && object.file.kind !== "image"
    && isWritingSourcePath(object.file.relativePath);
}
