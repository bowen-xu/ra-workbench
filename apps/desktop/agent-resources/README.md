# Application Agent resources

`skills/` owns the research application's bundled skill sources. The repository
root `.agents/skills/` is reserved for Codex developing this project. Application
skills must not be placed there or inferred from the repository working directory.

The native `agent::skills::bundled` adapter embeds the complete `edit-notes`
package in the application binary. Startup seeds it into the app configuration
root's `.agents/skills/edit-notes/`, where the existing catalog, exact revisions,
enablement, submission freezing and demand resource reads apply. No developer
skill paths, Codex private configuration or absolute paths are exposed to models.

Initialization is recorded once for this default. Existing same-path or same-name
packages are never overwritten, disabled policy is preserved, and deleting an initialized skill
does not reinstall it on restart. Installed skill upgrades use the existing
explicit, revision-bound import flow; updating embedded sources alone does not
replace a user's installed package. Keep new default packages and their receipts
explicit in the native adapter.

`edit-notes/references/text-view.md` is generated from the same versioned protocol
as native tool descriptions. Run `pnpm note:protocol` and `pnpm check:agent` to
generate and verify it. Do not maintain a second hand-written syntax definition.

Tools remain typed native/Rust registrations, including `agent/text_tools` and
the shared `packages/agent-core/src/tool_registry.rs`; skill text does not grant
tool access or introduce another execution owner.
