# Release Process

This file documents the repository-local workflow to follow when preparing an
OxiDNS Next release. It is maintainer-facing guidance, not end-user documentation.

The release contract spans `.github/workflows/release.yml`,
`.github/workflows/docker.yml`, `.github/workflows/custom-build.yml`, Cargo
manifests, packaging files, the upgrade asset selector, and the public docs.
Changing artifact names or bundle contents in only one of these places is a
release regression.

## 1. Build The Release Story From Tags

Start from the latest release tag and use the changes since that tag as the
source of truth for the release scope.

Recommended commands:

```bash
LATEST_TAG=$(git tag --list 'v*' --sort=-v:refname | head -n 1)
echo "$LATEST_TAG"
git log --oneline --decorate --no-merges "$LATEST_TAG"..HEAD
git diff --stat "$LATEST_TAG"..HEAD
git diff --name-only "$LATEST_TAG"..HEAD
```

Use the commit log and diff together:

- Summarize user-visible behavior, compatibility impact, operational changes,
  and bug fixes from `LATEST_TAG..HEAD`.
- Check touched subsystems before deciding whether the release is patch, minor,
  or major.
- Do not invent release-note items that are not visible in the commit range or
  the current diff.
- If the working tree contains release-prep edits, keep them separate in your
  reasoning from product changes since the previous tag.

## 2. Update Cargo Versions

Update the root package version for every release:

- `Cargo.toml` at the repository root, `[package].version`

If any crate under `crates/` has code changes since the latest release tag, bump
that crate's own `Cargo.toml` too:

- `crates/macros/Cargo.toml`
- `crates/proto/Cargo.toml`
- `crates/ripset/Cargo.toml`
- `crates/zoneparser/Cargo.toml`

Use path-level diffs to decide which crate versions need to change:

```bash
git diff --name-only "$LATEST_TAG"..HEAD -- crates/macros
git diff --name-only "$LATEST_TAG"..HEAD -- crates/proto
git diff --name-only "$LATEST_TAG"..HEAD -- crates/ripset
git diff --name-only "$LATEST_TAG"..HEAD -- crates/zoneparser
```

When a crate version changes:

- Update the crate's `[package].version`.
- Update any local dependency version declarations that refer to that crate,
  including root `Cargo.toml` path dependencies.
- Refresh `Cargo.lock` through a normal Cargo command such as `cargo check` or
  the release validation command.

Do not bump a workspace crate just because the root package is being released;
bump it only when that crate changed or its published dependency metadata must
change.

## 3. Generate Release Notes In Docs

Update both release-note files:

- `docs/docs/releases.md`
- `docs/i18n/en/docusaurus-plugin-content-docs/current/releases.md`

Follow the existing `ReleaseCard` format. For a new latest release:

- Insert the new card at the top of the matching month section, or add a new
  `## YYYY-MM` section if needed.
- Set the card version to the release tag, for example `v1.0.2`.
- Choose the badge from the semantic version impact, such as `Patch Release`,
  `Minor Release`, or `Major Release`.
- Use the intended release date in `YYYY-MM-DD` format.
- Move `defaultOpen` to the newest card only.
- Keep the Chinese file and English i18n file aligned in content and structure.

Use the established sections:

- Chinese: `版本定位`, `主要变更`, `配置与升级说明`
- English: `Release Scope`, `Changes`, `Compatibility and Upgrade Notes`

The content should be generated from the latest-tag-to-HEAD changes gathered in
step 1. The upgrade notes must mention:

- The root crate version and expected release tag.
- Whether existing configs can upgrade directly.
- Any new, renamed, or behavior-changing config fields.
- Any operational cautions, migration steps, or compatibility risks.

## 4. Prepare GitHub Release Notes

Update `RELEASE_NOTES.md` for the intended tag. This fixed file is
overwritten during every release preparation, while Git history retains the
previous versions. The reviewed release description therefore becomes part of
the tagged source instead of a manual post-publication edit. The first line
must be `# OxiDNS Next vX.Y.Z`; the tag workflow rejects a missing, empty, or
mismatched file.

Keep this file shorter than the full documentation release notes, but make it
complete enough for operators deciding whether to upgrade. The tag workflow
uses it as the reviewed GitHub Release body:

- `softprops/action-gh-release` publishes it without generated release-note
  sections, so every shipped claim must already be present in this file.

Use this standard Chinese template. A small number of emoji is allowed when it
improves scanability:

```markdown
# OxiDNS Next v1.3.0

## 🚀 发布概览

- 用一到两句话说明本次发布的定位、版本影响和最重要的变化。
- 说明适合升级的人群或主要收益。

## ✨ 主要亮点

- 重要功能、行为变化或兼容性改进。
- 关键 bug 修复、稳定性增强或运维体验改善。
- 如适用，补充 WebUI、打包、文档或平台相关变化。

## ⚠️ 升级说明

- 现有配置是否可以直接升级。
- 如有迁移步骤，在这里明确列出。
- 如有服务管理、WebUI、平台或配置兼容性风险，在这里说明。

## 📦 下载与校验

- 根据平台和 bundle 选择对应 archive。
- 替换生产环境二进制前，请使用 release assets 中的校验信息确认文件完整性。
```

Generation rules:

- Base the GitHub Release text on the same latest-tag-to-HEAD evidence from
  step 1 and the docs release notes from step 3.
- Do not include items that were not shipped in the tagged commit.
- Keep `Validation` limited to commands actually run for this release.
- Write the final GitHub Release body in Chinese.
- Mention breaking changes or config migrations in both `发布概览` and
  `升级说明`.
- Do not paste the full website release card verbatim; GitHub Release text
  should be concise and action-oriented.

Include a changelog link only when it materially helps operators; the workflow
does not append generated sections.

## 5. Confirm The Release Artifact Contract

The tag workflow publishes the following artifacts.

### Full bundle

Full archives include the binary, `config.yaml`, `LICENSE`, and WebUI files.

- Linux: `x86_64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`,
  `aarch64-unknown-linux-gnu`, `aarch64-unknown-linux-musl`,
  `i686-unknown-linux-musl`, `arm-unknown-linux-musleabihf`, and
  `armv7-unknown-linux-musleabihf`.
- macOS: `x86_64-apple-darwin` and `aarch64-apple-darwin`.
- FreeBSD: `x86_64-unknown-freebsd`.
- Windows: `x86_64-pc-windows-msvc`, `i686-pc-windows-msvc`, and
  `aarch64-pc-windows-msvc`.
- Debian packages: x86_64 and aarch64 GNU Linux targets.

Full archive names remain compatible with the upgrade selector:

```text
oxidns-next-<target>.tar.gz
oxidns-next-<target>.zip
```

### Slim bundles

- `minimal`: x86_64 and aarch64 Linux musl archives, using
  `config.minimal.yaml` and shipping no WebUI.
- `standard`: x86_64 and aarch64 Linux musl archives, including `config.yaml`
  and WebUI files.

Slim names include the bundle:

```text
oxidns-next-minimal-<target>.tar.gz
oxidns-next-standard-<target>.tar.gz
```

### Downstream publication

- Docker images are built from the published full musl archives for amd64 and
  arm64, then combined into multi-architecture manifests for Docker Hub and
  GHCR.
- The reusable custom-build workflow must keep target-to-runner, build-tool,
  archive naming, and WebUI packaging rules aligned with `release.yml`.

Before tagging, compare any workflow, feature, target, packaging, or upgrade
changes against this contract. If the contract intentionally changes, update
the workflows, upgrade selection tests, install/custom-build docs, and this
section together.

## 6. Validate Before Tagging

Run the repository gate before creating the release tag:

```bash
just check
```

Run the full feature matrix when the release includes Cargo feature, optional
dependency, bundle, protocol, or broad cfg changes:

```bash
just check-matrix
```

Also run these when the corresponding areas changed:

```bash
cd webui && pnpm typecheck
cd docs && npm run build
```

Also verify before tagging:

- `Cargo.toml` package version equals the intended `vX.Y.Z` tag without the
  leading `v`.
- `Cargo.lock` contains the intended root and changed workspace crate versions.
- `RELEASE_NOTES.md` starts with the matching version heading and contains only
  the curated notes.
- `oxidns build-info` reports the expected bundle/features for any locally
  built release candidate.
- `oxidns check` accepts the packaged full and minimal example configs under
  their corresponding bundles.
- `cargo package --locked` succeeds when package metadata or crate contents
  changed.
- No release-note claim depends on uncommitted working-tree changes.

## 7. Hand Off For Commit And Tag

Do not automatically commit, tag, or push as part of release preparation.
After versions, docs release notes, GitHub Release text, and validation are
complete, hand the final state to the maintainer with:

- A concise summary of the release-prep changes.
- The validation commands that were actually run.
- The reviewed `RELEASE_NOTES.md` content.
- Suggested manual commit and tag commands.

Suggested commit message:

```text
chore(release): prepare v1.0.2
```

Suggested tag command after the maintainer has reviewed and committed the
release-prep changes:

```bash
git tag vX.Y.Z
```

The GitHub release workflow is triggered by pushing tags matching `v*`.
The maintainer should only push the tag after reviewing the release-prep commit
and release-notes file.

Before pushing, verify the tag points at the reviewed release commit:

```bash
git show --no-patch --decorate vX.Y.Z
```

## 8. Verify Publication

Do not consider the release complete when the tag is pushed. Wait for and check
these workflow stages:

1. WebUI build.
2. Full and slim archive matrices.
3. GitHub Release publication.
4. amd64/arm64 Docker builds and multi-architecture manifests.

Inspect the release and download at least one representative archive:

```bash
gh release view vX.Y.Z
release_tmp="$(mktemp -d)"
gh release download vX.Y.Z --pattern 'oxidns-next-x86_64-unknown-linux-musl.tar.gz' --dir "$release_tmp"
tar -tzf "$release_tmp/oxidns-next-x86_64-unknown-linux-musl.tar.gz"
```

Verify that:

- Expected full, slim, and Debian assets exist with correct names.
- The archive contains the expected config, license, and WebUI policy for its
  bundle.
- The extracted binary reports the intended version and build bundle.
- The example config validates with that binary.
- GitHub reports a digest for downloadable assets; the self-upgrade path relies
  on the release asset digest for SHA256 verification.
- Docker Hub and GHCR expose the expected version and architecture manifests.
- The repository/tag metadata are correct.
- The final GitHub Release body, `RELEASE_NOTES.md`, and both documentation
  release cards agree.

Keep a short publication record with the tag commit, workflow URL, validation
commands, and any platform not manually smoke-tested.

## 9. Failed Release And Rollback

- If local validation fails before tagging, fix the cause on a new commit and
  tag only after the reviewed commit is ready.
- If a pushed-tag workflow fails before publication and a source change is
  required, do not move the tag automatically. Explicitly decide whether to
  withdraw an unpublished tag or advance to a patch release, and record the
  decision.
- If a transient job fails for an already pushed tag but no source change is
  needed, rerun the workflow for the same tag/commit. Do not move the tag to a
  different commit silently.
- If published artifacts are incomplete, avoid announcing the release until
  the exact-tag workflow has completed and the artifact contract is verified.
- If the shipped product is defective, publish a patch release. Do not replace
  versioned assets with different binaries under the same tag.
- crates.io versions cannot be overwritten. Yank only when distribution is
  actively harmful, explain why, and follow with a corrected version.
- For bad Docker publication, preserve version-tag immutability; publish a
  corrected patch and repair moving aliases such as `latest` only with an
  explicit incident note.
- Deployment rollback follows `ai/operations-runbook.md`: restore the previous
  binary, WebUI, and config, then repeat health and DNS verification.

After any release incident, record whether the prevention belongs in CI,
packaging smoke tests, the artifact contract, or the pre-tag checklist.
