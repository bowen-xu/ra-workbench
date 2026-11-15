# ra-workbench

Core workspace, desktop adapters and physical execution.

Source extracted from research-assistant 10d1f488ce8ce209d8c8f27b016218805877c0ff.

The integration repository pins this project as a Git submodule.
Its `pnpm build -- workbench` command builds the selected project using the existing isolated build pipeline.
Public SDK packages retain their original package names and versions.
