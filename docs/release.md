# Release process

`knect` ships signed, multi-architecture release artifacts from
`.github/workflows/release.yml`. A release is one tag and one immutable set of
assets: archives, checksums, SPDX and CycloneDX SBOMs, keyless Cosign bundles,
GitHub Artifact Attestations, a multi-arch image, a Helm chart, a Homebrew
formula, and a krew manifest.

The workflow has no unsigned publish path. For a real tag it refuses to run
when `Dockerfile` or `charts/knectd/Chart.yaml` is missing, and it will not
create the GitHub release unless the image build, image signing, blob signing,
and attestations all succeed.

## Artifact names

`TAG` includes the leading `v` (for example `v1.0.0` or `v1.0.0-rc.1`).

| Artifact | Name |
| --- | --- |
| Linux amd64 archive | `knect_${TAG}_linux_amd64.tar.gz` |
| Linux arm64 archive | `knect_${TAG}_linux_arm64.tar.gz` |
| macOS amd64 archive | `knect_${TAG}_darwin_amd64.tar.gz` |
| macOS arm64 archive | `knect_${TAG}_darwin_arm64.tar.gz` |
| Checksums | `knect_${TAG}_checksums.txt` |
| Binary SBOM | `knect_${TAG}_<os>_<arch>.spdx.json` and `.cyclonedx.json` |
| Image SBOM (linux/amd64) | `knect_${TAG}_image.spdx.json` and `.cyclonedx.json` |
| Blob signature bundle | `<artifact>.sigstore.json` |
| Helm chart | `knectd-<version>.tgz` (`<version>` excludes the leading `v`) |
| Image | `ghcr.io/OWNER/knectd:${TAG}`, `:<version>`, `:latest` (stable only), `:sha-<sha>` |

Every archive contains `knect`, `knectd`, `LICENSE`, and `README.md`.
Homebrew installs both binaries. krew installs `knect` only; `knectd` comes
from Homebrew or an archive.

## Templates

- `homebrew/knect.rb.tpl` is rendered by `hack/release/render-homebrew.sh` from
  `knect_${TAG}_checksums.txt`.
- `install/krew/knect.yaml.tpl` is rendered by `hack/release/render-krew.sh`
  from the same checksums file. The rendered manifest is what
  `rajatjindal/krew-release-bot` submits to `krew-index`.

Both renderers fail when any of the four archives is absent from the checksums
file, so template URLs cannot drift from published artifact names.

## Pipeline stages

1. **meta** validates the tag, resolves the image/tap, checks required secrets,
   and verifies that `Dockerfile` and `charts/knectd` exist for a publish.
2. **verify** runs `cargo fmt --check`, clippy with `-D warnings`, and the
   workspace tests.
3. **binaries** builds four targets:
   - `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` with
     `cargo-zigbuild` (Zig is pinned and checksum-verified in the workflow)
   - `x86_64-apple-darwin` on `macos-15-intel`
   - `aarch64-apple-darwin` on `macos-15`

   Each job packages `knect`, `knectd`, `LICENSE`, and `README.md`.
4. **sboms** runs Syft over every archive and emits SPDX JSON and CycloneDX
   JSON.
5. **docker** builds and pushes `linux/amd64` and `linux/arm64` on GHCR,
   signs the image digest with keyless Cosign, generates image SBOMs, and
   pushes provenance and SBOM attestations with `actions/attest`.
6. **release** packages and pushes the Helm chart, generates the checksums,
   renders the Homebrew formula and krew manifest, signs every release blob,
   creates GitHub Artifact Attestations, verifies signatures and attestations,
   publishes the GitHub release, and opens the krew-index PR.
7. **homebrew** commits the rendered formula to the default branch of the tap.

Homebrew and krew are skipped for prereleases. The Helm chart, image, archives,
SBOMs, signatures, and attestations are still produced for an RC.

## Required repository settings and secrets

- Repository workflow permissions must allow the release job to request `contents: write` and
  `packages: write`; organization policy must not downgrade those scopes.
- Actions must be allowed to create releases and packages.
- The GHCR package must allow the repository's `GITHUB_TOKEN` to write. The
  workflow uses `packages: write`; no registry PAT is stored.
- `HOMEBREW_TAP_TOKEN` is required for a stable publish. It is a fine-grained
  or classic token with `contents: write` on `OWNER/homebrew-tap` (or the tap
  passed as the `homebrew_tap` input). Missing token on a stable publish fails
  in the `meta` job before any build starts.
- `GITHUB_TOKEN` is provided by Actions and is used for GHCR, release upload,
  GitHub Artifact Attestations, and the krew bot API call.
- `KREW_RELEASE_BOT_WEBHOOK_URL` is optional. The krew bot defaults to its
  public webhook, which opens the PR against `kubernetes-sigs/krew-index`.
- The first krew submission must be merged manually. The bot only updates an
  existing plugin entry.

`OWNER`/`REPO` are resolved from `${{ github.repository }}` at runtime. No
organization-specific values are hard-coded in the workflow.

## Release checklist

1. Confirm the workspace version in `Cargo.toml` matches the intended release
   (for example `1.0.0-rc.1` or `1.0.0`). Cargo's version is what
   `knect --version` and `knectd --version` print.
2. Confirm `Dockerfile` exists and builds the daemon/image payload expected by
   `charts/knectd`.
3. Confirm `charts/knectd/Chart.yaml` exists and lints:
   `helm lint charts/knectd && helm template knectd charts/knectd`.
4. Run `make release-check` and then the dry run first (below).
5. Tag and push from the commit that passed CI:
   `git tag -a v1.0.0-rc.1 -m "knect v1.0.0-rc.1" && git push origin v1.0.0-rc.1`
6. Watch the **release** workflow. All jobs must be green.
7. Verify the published assets with the commands in this document.
8. For a stable release, confirm the Homebrew tap commit and the krew-index PR.

## Dry run

Use **Actions -> release -> Run workflow** with:

- `tag`: a synthetic tag such as `v0.0.0-dryrun`, or a real future tag.
- `dry_run`: `true` (default).

A dry run builds every archive and SBOM, renders the Homebrew formula and krew
manifest, uploads them in the `release-assets` workflow artifact, and writes a
summary. It does **not** log in to a registry, push an image, sign, attest,
create a GitHub release, commit to the tap, or call the krew bot. If
`Dockerfile` or `charts/knectd` is absent the dry run warns and skips that
stage; a publish fails instead.

To exercise the publish path from the UI, set `dry_run` to `false` and provide
a real `tag`. This is only safe after the checklist above.

## Release candidate

Tags containing a hyphen are prereleases automatically. `v1.0.0-rc.1`:

- produces `knect_v1.0.0-rc.1_*` archives, SBOMs, signatures, and attestations;
- creates a GitHub prerelease;
- tags the image `v1.0.0-rc.1` and `1.0.0-rc.1`, but not `latest`;
- pushes `knectd-1.0.0-rc.1.tgz` to `oci://ghcr.io/OWNER/charts`;
- skips the Homebrew tap commit and the krew-index PR.

## Signature and attestation model

- Blob signatures use keyless Cosign through GitHub OIDC and Fulcio/Rekor.
  Each release payload (archives, SBOMs, Helm chart, checksums) has a
  `<artifact>.sigstore.json` bundle. The checksums file covers every payload.
- The image is signed by digest, recursively, so the index and child manifests
  are signed.
- `actions/attest` creates GitHub Artifact Attestations for every subject in
  the checksums file and pushes image provenance and SBOM attestations to GHCR.
  `artifact-metadata: write` is required for the storage record.

### GHCR and Cosign v3

Cosign v3 defaults to the new Sigstore bundle format and OCI 1.1 referrers.
GHCR does not accept that path for image signatures yet, so image signing and
verification pass `--new-bundle-format=false --use-signing-config=false`.
These are explicitly legacy flags, not the long-term default. Attestations are
pushed with `actions/attest` `push-to-registry: true`, which uses the registry
path GHCR supports. Revisit the flags when GHCR supports the v3 bundle format.

## Verification

Set these once:

```bash
TAG=v1.0.0
VERSION="${TAG#v}"
REPO=OWNER/REPO
IMAGE="ghcr.io/${REPO%%/*}/knectd"
DIGEST="$(docker buildx imagetools inspect "${IMAGE}:${TAG}" --format '{{json .Manifest.Digest}}' | tr -d '"')"
```

Checksums:

```bash
sha256sum -c "knect_${TAG}_checksums.txt"
```

Blob signatures:

```bash
cosign verify-blob \
  --bundle "knect_${TAG}_checksums.txt.sigstore.json" \
  --certificate-identity-regexp "^https://github.com/${REPO}/.github/workflows/release.yml@.*$" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  "knect_${TAG}_checksums.txt"
```

Repeat for any archive or SBOM by swapping the file and bundle names.

Image signature:

```bash
cosign verify \
  --new-bundle-format=false \
  --certificate-identity-regexp "^https://github.com/${REPO}/.github/workflows/release.yml@.*$" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  "${IMAGE}:${TAG}"
```

GitHub Artifact Attestations:

```bash
gh attestation verify "knect_${TAG}_linux_amd64.tar.gz" \
  --repo "${REPO}" \
  --signer-workflow "${REPO}/.github/workflows/release.yml"

gh attestation verify "knect_${TAG}_linux_amd64.spdx.json" \
  --repo "${REPO}" \
  --signer-workflow "${REPO}/.github/workflows/release.yml"
```

Image SBOM attestation:

```bash
cosign verify-attestation \
  --type spdxjson \
  --certificate-identity-regexp "^https://github.com/${REPO}/.github/workflows/release.yml@.*$" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  "${IMAGE}@${DIGEST}"
```

Install checks:

```bash
brew install "${REPO%%/*}/tap/knect"
kubectl krew install knect
helm pull "oci://ghcr.io/${REPO%%/*}/charts/knectd" --version "${VERSION}"
```

## Rollback

A release is immutable; publish a new tag rather than rewriting one.

- **GitHub release**: delete the release and the tag. Existing downloads stop
  resolving; signatures and attestations remain in the transparency log.
- **GHCR image**: delete the version tags (`${TAG}`, `${VERSION}`, `latest`).
  Digests are content-addressed and cannot be deleted; the audit trail remains.
- **Helm**: delete the chart tag from the GHCR chart repository and publish a
  corrected version.
- **Homebrew**: revert the formula commit in the tap. Users who already
  installed the bad version keep it until they upgrade.
- **krew**: close the bot PR, or revert the krew-index merge and submit a fixed
  manifest.
- **Checksums/SBOMs/signatures**: keep the assets for audit, but mark the
  release as superseded in the release notes.

## Troubleshooting

- `meta` fails with a tag error: use `vMAJOR.MINOR.PATCH` or a prerelease with
  a hyphen, for example `v1.0.0-rc.1`.
- `meta` fails on a missing secret: add `HOMEBREW_TAP_TOKEN` or set
  `publish_homebrew` to `false` for a non-tap publish.
- `meta` fails on `Dockerfile`: a publish must not create an unsigned image;
  land the Dockerfile.
- `meta` fails on `charts/knectd`: land the chart, or set `publish_helm` to
  `false` for a publish that intentionally skips the chart.
- `docker` fails in `cosign sign`: confirm `id-token: write`,
  `packages: write`, and the legacy flags above. Re-run the job after the
  registry recovers; the digest does not change if the build is cache-hit.
- `release` fails with a packaged-chart image mismatch: set
  `charts/knectd/values.yaml` `image.repository` to `ghcr.io/OWNER/knectd`.
- `release` fails in `gh attestation verify`: confirm the run has
  `attestations: write` and `artifact-metadata: write`, and that the signer
  workflow path matches `.github/workflows/release.yml`.
- `homebrew` fails with a push rejection: the tap branch may be protected or
  the token lacks `contents: write`. Fix the token or branch rules and re-run
  the job.
- krew bot fails: verify the release is not a prerelease, that the rendered
  manifest is in `dist/render/krew/knect.yaml`, and that the plugin already
  exists in `krew-index`.
