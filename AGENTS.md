# Agent entry point

Read [docs/STATUS.md](docs/STATUS.md) before changing the repository. It identifies the active phase, verified state, blockers, and next task.

Then read only the documents required for the task:

- Product behavior: [docs/PRD.md](docs/PRD.md)
- Architecture and interfaces: [docs/TECHNICAL_SPEC.md](docs/TECHNICAL_SPEC.md)
- Work status: [docs/BUILD_CHECKLIST.md](docs/BUILD_CHECKLIST.md)
- Security boundaries: [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md)
- Prior decisions: [.audit/recovery-control-room.tsv](.audit/recovery-control-room.tsv)

Use Bun for all JavaScript package and script commands. Use Cargo for Rust. Do not introduce npm, pnpm, or Yarn lockfiles.

Complete the active phase before starting another phase. Run its verification commands, update `docs/STATUS.md`, write its report under `docs/phase-reports/`, and append significant decisions to the audit log.

The Rust backend owns domain and authorization rules. TypeScript may validate wire data and manage browser state, but it must not duplicate recovery rules.

Use `apply_patch` for source and documentation edits. Preserve unrelated work and avoid destructive Git commands.

