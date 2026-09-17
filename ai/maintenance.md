# Maintenance Guide

This document defines recurring repository maintenance outside feature work and
release preparation. Release-specific versioning and publication remain in
`ai/release-process.md`; vulnerability handling remains in `SECURITY.md`.

## Maintenance Goals

- Keep supported bundles and platforms buildable.
- Keep dependency, feature, docs, WebUI, and packaging representations aligned.
- Reduce obsolete code and unsafe compatibility assumptions deliberately.
- Preserve reproducible builds and operational rollback paths.
- Prevent routine upgrades from becoming large mixed-risk changes.

## Toolchain Contract

- Rust edition: 2024.
- Stable Rust: normal builds, tests, docs, and release binaries.
- Nightly Rust: rustfmt and strict Clippy commands because `rustfmt.toml` uses
  unstable features.
- WebUI CI: Node.js 22 and pnpm 10.
- Docs CI: Node.js 20 and npm with `package-lock.json`.
- `Cargo.lock`, `webui/pnpm-lock.yaml`, and `docs/package-lock.json` are
  reproducibility artifacts and must be updated with their manifests.

Do not change toolchain versions in only one workflow. Check local guidance,
all CI workflows, installer/build documentation, and reusable custom builds.

## Dependency Update Policy

Dependabot opens weekly grouped updates for:

- Cargo dependencies.
- GitHub Actions.
- Docker dependencies.

WebUI and docs JavaScript dependencies require explicit maintenance because
they are not covered by Dependabot.

For each dependency update:

1. Read upstream release notes for behavior, feature, MSRV, platform, security,
   and default-feature changes.
2. Identify every direct and transitive usage. Use `cargo tree -i <crate>` for
   Rust dependencies when ownership is unclear.
3. Keep unrelated major upgrades in separate commits/PRs.
4. Preserve optional dependency gating; a new optional crate must not leak into
   `minimal` unless intentionally required.
5. Update the manifest and lockfile through the native package manager.
6. Run affected focused tests, then the validation required below.
7. Call out generated-code, wire-format, TLS, database, or persistence changes.

The `mikrotik-rs` dependency is currently mapped to the OxiDNS-maintained
`oxidns-mikrotik-rs` package because the upstream Tokio response-channel fix has
not been released. Treat updates to that package as source changes, review the
exact release, and run RouterOS transport/manager tests.

## Validation by Dependency Type

### Rust patch/minor updates

```bash
just check
```

Use `just check-matrix` for optional dependencies, feature graph changes,
proc-macros, async runtime/networking, TLS/HTTP/QUIC, serialization, or platform
integration updates.

### Rust major updates

- Update one subsystem at a time.
- Inspect public API and behavior migrations rather than relying only on a
  successful compile.
- Run the full feature matrix and review affected request paths and resource
  bounds for hot-path libraries.
- Let Linux, Windows, and macOS CI complete before merge.

### WebUI dependencies

```bash
cd webui
pnpm install
pnpm typecheck
pnpm lint
pnpm test
pnpm build
```

Use the repository's pinned pnpm major. Review Next.js, React, Tailwind, Radix,
and build-output changes for runtime and static export impact.

### Docs dependencies

```bash
cd docs
npm install
npm run build
```

Prefer `npm ci` when validating the committed lockfile exactly.

### GitHub Actions and Docker

- Review permission changes and action input/output changes.
- Keep release and reusable custom-build target/packaging logic aligned.
- For Docker base/runtime changes, build locally without push and run the image
  smoke checks used by the workflow.

## Feature and Bundle Hygiene

At least periodically, and whenever features change:

```bash
just check-each-feature
just check-minimal
just check-standard
just check-full
```

Use `just check-powerset` before or after broad feature-graph changes when local
time permits; nightly CI provides the recurring depth-2 sweep.

Check that:

- Public features follow category naming rules.
- Private `_` aggregators are not documented as user-facing switches.
- Optional dependencies are reachable only from intended features.
- Bundle membership matches `ai/plugin-dev.md`, custom-build documentation,
  build info, and release packaging.
- Disabled-feature fallback paths remain warning-free.

CI also runs `cargo-shear`. Review removals manually: proc-macro, build-script,
platform-only, and feature-only dependencies may not look used in the active
configuration.

## Workspace Crate Maintenance

The workspace includes the root package and crates for macros, protocol types,
ripset, and zone parsing.

- Keep each crate's manifest metadata and version internally consistent.
- Update root path dependency version requirements when a child crate version
  changes.
- Do not bump every crate for every OxiDNS release; bump a crate when its code
  or published dependency contract changed.
- Run workspace tests/docs after shared protocol or proc-macro changes.
- Verify `cargo publish --dry-run` for any crate intended for publication.

The root release workflow publishes the root crate. Publishing child crates or
changing their publication order requires an explicit release workflow change.

## Code Health

Recurring cleanup should look for:

- Modules that combine unrelated config, model, lifecycle, metrics,
  persistence, and protocol responsibilities.
- Duplicate parsing or transport abstractions.
- `infra -> plugin` dependency regressions.
- Unbounded queues, maps, retry loops, or background tasks.
- Blocking work or high-cardinality logging/metrics in request paths.
- Deprecated config aliases whose removal window has passed.
- Platform cfg branches not exercised by local development.
- Tests that depend on fixed ports, public networks, sleeps, or global state.
- Stale examples and mismatched Chinese/English documentation.

Structural cleanup must preserve behavior unless the PR explicitly declares a
behavior change. Use `ai/architecture.md` for placement decisions.

## Configuration and Persistence Evolution

- Prefer additive optional fields with defaults that preserve old behavior.
- Reject unknown fields where silent typos would be dangerous.
- Keep error messages tied to config paths and plugin tags.
- Document renamed/removed fields and provide an upgrade path.
- Version or detect persistence formats when incompatible evolution is
  possible.
- Test old fixtures, corrupt/truncated data, and partial write recovery.
- Do not remove compatibility code solely because current tests generate only
  the newest format.

Use `ai/change-impact-matrix.md` to synchronize WebUI schemas, examples, API
payloads, user docs, and release notes.

## Documentation Maintenance

Quarterly or before a significant release, check:

- `AGENTS.md` matches the actual top-level and package structure.
- Every file listed in `ai/README.md` exists and remains authoritative.
- Plugin types, config fields, metrics, and examples match between Rust,
  Chinese docs, English docs, WebUI definitions, and locales.
- Commands match the current Clap subcommand structure.
- Release and operations documents match workflow and service behavior.
- Historical release notes remain historical; do not rewrite old claims merely
  because paths or current behavior changed later.

## Suggested Cadence

### Weekly

- Triage Dependabot and security advisories.
- Review CI failures, nightly feature powerset, and flaky-test evidence.
- Check release/operations issues for recurring failure patterns.

### Monthly

- Update WebUI/docs dependencies in bounded batches.
- Review outdated direct dependencies and patched Git sources.
- Run or inspect bundle/feature matrix results.
- Review warnings, deprecations, and platform compatibility notices.

### Before a minor or major release

- Audit config/API/persistence compatibility.
- Review bundle contents and release target matrix.
- Re-run relevant performance baselines for hot-path changes.
- Validate upgrade and rollback documentation.
- Follow the complete `ai/release-process.md` workflow.

## Maintenance Handoff

Maintenance PRs should state:

- Why the update is needed now.
- Direct and transitive scope.
- Feature, platform, config, persistence, and performance risk.
- Lockfiles and generated artifacts changed.
- Exact commands run.
- Any CI/platform result still required before merge.
