# syntax=docker/dockerfile:1.7

# Multi-stage build for knectd/knect:
#   1. webui  - build the Svelte bundle with Node.
#   2. rust   - compile static musl binaries with cargo-zigbuild.
#   3. runtime - distroless static image with no shell, package manager, or libc.
#
# Base images are pinned by tag; bump the ARGs deliberately. `latest` is never
# used. The WebUI and Rust stages run on the build platform because zigbuild
# cross-compiles, so a buildx multi-arch build does not need QEMU for Rust.

ARG NODE_VERSION=26.12.0
ARG RUST_VERSION=1.98.1
ARG ZIG_VERSION=0.15.2
ARG CARGO_ZIGBUILD_VERSION=0.24.0
ARG DISTROLESS_IMAGE=gcr.io/distroless/static-debian12:nonroot

FROM --platform=$BUILDPLATFORM node:${NODE_VERSION}-bookworm-slim AS webui
WORKDIR /src/webui
COPY webui/package.json webui/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm,sharing=locked \
    npm ci --no-audit --no-fund
COPY webui/ ./
RUN npm run build

FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-bookworm AS rust-builder
ARG ZIG_VERSION
ARG CARGO_ZIGBUILD_VERSION
ARG TARGETARCH

# zig supplies the cross C/C++ toolchain; curl and xz fetch and unpack zig.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        xz-utils \
    && rm -rf /var/lib/apt/lists/*

RUN set -eux; \
    arch="$(uname -m)"; \
    case "${arch}" in \
        x86_64|aarch64) ;; \
        *) echo "unsupported build architecture: ${arch}" >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://ziglang.org/download/${ZIG_VERSION}/zig-${arch}-linux-${ZIG_VERSION}.tar.xz" \
        -o /tmp/zig.tar.xz; \
    mkdir -p /opt/zig; \
    tar -xJf /tmp/zig.tar.xz -C /opt/zig --strip-components=1; \
    rm /tmp/zig.tar.xz; \
    ln -s /opt/zig/zig /usr/local/bin/zig; \
    zig version

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo install cargo-zigbuild --version "${CARGO_ZIGBUILD_VERSION}" --locked

RUN set -eux; \
    case "${TARGETARCH}" in \
        amd64) target=x86_64-unknown-linux-musl ;; \
        arm64) target=aarch64-unknown-linux-musl ;; \
        *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    echo "${target}" > /tmp/rust-target; \
    rustup target add "${target}"

WORKDIR /src
# .dockerignore keeps host build output, node_modules, and VCS data out. The
# committed WebUI bundle is replaced by the freshly built one below so the
# binary always embeds the same source as the image.
COPY . .
RUN rm -rf crates/knectd/assets/webui
COPY --from=webui /src/webui/dist crates/knectd/assets/webui
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    set -eux; \
    target="$(cat /tmp/rust-target)"; \
    cargo zigbuild --release --locked --target "${target}" \
        -p knectd -p knect-tui; \
    mkdir -p /out /out/cache; \
    cp "target/${target}/release/knectd" "target/${target}/release/knect" /out/; \
    chmod 0755 /out/knectd /out/knect

FROM ${DISTROLESS_IMAGE} AS runtime

ARG VERSION=dev
LABEL org.opencontainers.image.title="knectd" \
      org.opencontainers.image.description="Read-only Knative topology observer with embedded WebUI" \
      org.opencontainers.image.source="https://github.com/enchantednatures/knect" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.version="${VERSION}"

COPY --from=rust-builder /out/knectd /usr/local/bin/knectd
COPY --from=rust-builder /out/knect /usr/local/bin/knect
COPY --from=rust-builder --chown=65532:65532 /out/cache /var/cache/knect

# In-cluster defaults: bind all interfaces (the Service fronts it), keep the
# snapshot cache on a writable mount, and emit structured logs.
ENV KNECT_DAEMON_HTTP_ADDR=0.0.0.0:7420 \
    KNECT_DAEMON_SOCKET_PATH=/tmp/knectd.sock \
    KNECT_DAEMON_CACHE_DIR=/var/cache/knect \
    KNECT_ALLOW_REMOTE=1 \
    KNECT_DAEMON_LOG_FORMAT=json

EXPOSE 7420 7421

# distroless `nonroot` is UID/GID 65532. The numeric form works even if the
# image's /etc/passwd is overridden by an orchestrator.
USER 65532:65532

ENTRYPOINT ["/usr/local/bin/knectd"]
