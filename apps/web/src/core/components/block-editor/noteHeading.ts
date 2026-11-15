import { createHeadingBlockSpec, type BlockImplementation } from "@blocknote/core";

/** Keep schema input rules, shortcuts and command menus on the same note policy. */
export const NOTE_HEADING_LEVELS = [1, 2, 3, 4] as const;
const heading = createHeadingBlockSpec({ levels: NOTE_HEADING_LEVELS });
const propSchema = {...heading.config.propSchema, isToggleable: {default: false, optional: true} as const};
export const noteHeadingSpec = {
  ...heading,
  config: {
    ...heading.config,
    // The pinned factory types this enabled prop as possibly undefined, which
    // violates PropSchema under exactOptionalPropertyTypes. Declare it explicitly.
    propSchema,
  },
  implementation: heading.implementation as unknown as BlockImplementation<"heading", typeof propSchema, "inline">,
};
