# Agency — Design Handoff

High-fidelity UI mockups + design spec for the **Agency** desktop app
(local, multi-agent terminal orchestrator · Tauri + Rust core + web UI).

## Contents

| File | What it is |
|------|------------|
| `Agency-Design-Handoff.html` | **Start here.** Full design spec — system, screens, components, states, dev notes. Open in a browser; print to PDF to share. |
| `Agency v2.dc.html` | **Recommended** mockup. Git Review side-panel in Agents, collapsible sidebar + focus agent-list, foldable project tree, project-switchable git history. |
| `Agency.dc.html` | Original mockup, kept for comparison. |
| `support.js` | Runtime that renders the `.dc.html` files. Keep it beside them. |

## Viewing the mockups

Open `Agency v2.dc.html` in a modern browser **with `support.js` in the same folder**.
Everything is clickable:

- Toggle **Grid / Focus** and the **Review** side-panel in the Agents header
- Open **Source Control** — stage files, view per-hunk diffs, switch history projects (A/W/M/D pills)
- Expand a **project** in the sidebar and click an agent to jump to its Focus view
- Collapse the **sidebar** (left-panel icon) and the **focus agent rail** (« / »)
- Open the **New task** and **Approve & merge** modals

## Notes

- These are visual + interaction mockups. Terminal output, diffs, and commit data are realistic placeholders.
- Fonts: Geist (UI) + JetBrains Mono (code). Bundle with the app for offline use.
- Palette: Catppuccin Mocha — see the spec for tokens and semantic mappings.

_v0.3.1 · 2026-06-19_
