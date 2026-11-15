/** Core owns Git worktree execution and runtime routing. Consumers receive
 * only the existing manuscript-scoped list/create/remove capability. */
export {getGitWorktreeGateway} from "../services/gitWorktreeGateway";
export type {GitWorktreeGateway, ManuscriptGitWorktree} from "@research-assistant/data/writing";
