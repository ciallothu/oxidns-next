# AI Project Notes

This directory is reserved for project-maintained AI notes that do not need a
tool-mandated path.

Current files:

- `architecture.md`: internal module ownership, dependency direction,
  lifecycle, and hot-path boundaries.
- `change-impact-matrix.md`: cross-surface synchronization and validation
  triggers for common change types.
- `maintenance.md`: recurring dependency, toolchain, feature, workspace, and
  documentation maintenance.
- `operations-runbook.md`: deployment preflight, health, diagnosis, upgrade,
  and rollback procedures.
- `performance.md`: hot-path, profiling, resource-safety, and performance
  engineering guidance.
- `plugin-dev.md`: plugin architecture, registration, feature-gating, testing,
  and documentation synchronization guidance.
- `release-process.md`: maintainer-facing release preparation workflow.
- `testing-strategy.md`: local validation ladder, CI parity, feature matrices,
  network tests, and DNS correctness rules.
- `webui.md`: WebUI-specific agent guidance.

Fixed-position files stay where tools discover them:

- `AGENTS.md` stays at the repository root and contains the canonical
  repository instructions.
- `CLAUDE.md` stays at the repository root for Claude discovery.
- `.claude/launch.json` stays under `.claude/` for Claude launch integration.

Put new AI-facing prompts and operating notes here unless a tool requires a
specific location.

## Content Policy

Keep these documents durable across releases. They should define project
contracts, decision criteria, repeatable workflows, and operational knowledge.
Do not use them to track planned features, target versions, task progress,
temporary migration steps, or one-off refactoring sequences; keep that work in
issues, milestones, pull requests, or the project roadmap.
