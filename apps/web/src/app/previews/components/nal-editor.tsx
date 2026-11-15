import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { NalEditorPreview } from "./NalEditorPreview";
import "../../../core/styles.css";

const root = document.getElementById("root");
if (!root) throw new Error("NAL_EDITOR_PREVIEW_ROOT_MISSING");
createRoot(root).render(<StrictMode><NalEditorPreview /></StrictMode>);
