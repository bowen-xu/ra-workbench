/** Fixed Core Git version capabilities. UI workflows and restore-proposal
 * application remain with the writing host; no raw command execution is exposed. */
export {getManuscriptVersionGateway} from "../services/gitVersionGateway";
export type {ManuscriptVersionCommit, ManuscriptVersionHistory, ManuscriptVersionGateway,
  GitRestoreScope, ManuscriptGitRestoreProposal, GitRepositoryView, ManuscriptGitBranches,
  ManuscriptVersionDiff, HistoricalVersionFile} from "@research-assistant/data/writing";
