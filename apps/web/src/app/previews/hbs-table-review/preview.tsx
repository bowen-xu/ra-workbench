import { createRoot } from "react-dom/client";
import type { NotesGateway } from "@research-assistant/data/notes";
import { NotesProvider } from "../../../core/features/notes/NotesClient";
import { TableReviewPreview } from "./TableReviewPreview";
import { ApplicationKeyboardPolicy } from "../../../core/runtime/ApplicationKeyboardPolicy";
import "../../../core/styles.css";

const gateway = new Proxy({}, { get: (_target, command) => async () => {
  throw new Error(`TABLE_PREVIEW_UNSUPPORTED：此内存示例未连接 ${String(command)}。`);
} }) as NotesGateway;
const root = document.getElementById("root");
if (!root) throw new Error("TABLE_PREVIEW_ROOT_MISSING");
createRoot(root).render(<NotesProvider gateway={gateway}><ApplicationKeyboardPolicy /><TableReviewPreview/></NotesProvider>);
