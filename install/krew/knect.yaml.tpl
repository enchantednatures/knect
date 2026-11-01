# SPDX-License-Identifier: Apache-2.0
# license: Apache-2.0
# Rendered by hack/release/render-krew.sh before krew-release-bot submits it.
apiVersion: krew.googlecontainertools.github.com/v1alpha2
kind: Plugin
metadata:
  name: knect
spec:
  version: @@TAG@@
  homepage: https://github.com/@@REPOSITORY@@
  shortDescription: Explore Knative topology from kubectl
  description: |
    knect opens a terminal topology explorer for Knative Serving, Eventing,
    and the Kubernetes resources they depend on. It talks to a knectd daemon
    over a Unix socket or an HTTP endpoint and can inspect graph details,
    mutation attribution, and CloudEvent taps.

    The plugin and daemon are distributed under the Apache License 2.0.
  caveats: |
    knect requires a running knectd daemon. Install knectd with Homebrew or
    from the release archives, then run `knectd` before invoking `kubectl knect`.
  platforms:
    - selector:
        matchLabels:
          os: linux
          arch: amd64
      uri: https://github.com/@@REPOSITORY@@/releases/download/@@TAG@@/knect_@@TAG@@_linux_amd64.tar.gz
      sha256: "@@SHA256_LINUX_AMD64@@"
      bin: knect
      files:
        - from: knect
          to: .
    - selector:
        matchLabels:
          os: linux
          arch: arm64
      uri: https://github.com/@@REPOSITORY@@/releases/download/@@TAG@@/knect_@@TAG@@_linux_arm64.tar.gz
      sha256: "@@SHA256_LINUX_ARM64@@"
      bin: knect
      files:
        - from: knect
          to: .
    - selector:
        matchLabels:
          os: darwin
          arch: amd64
      uri: https://github.com/@@REPOSITORY@@/releases/download/@@TAG@@/knect_@@TAG@@_darwin_amd64.tar.gz
      sha256: "@@SHA256_DARWIN_AMD64@@"
      bin: knect
      files:
        - from: knect
          to: .
    - selector:
        matchLabels:
          os: darwin
          arch: arm64
      uri: https://github.com/@@REPOSITORY@@/releases/download/@@TAG@@/knect_@@TAG@@_darwin_arm64.tar.gz
      sha256: "@@SHA256_DARWIN_ARM64@@"
      bin: knect
      files:
        - from: knect
          to: .
