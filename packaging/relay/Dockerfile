# Build from the repository root:
#   docker build -f packaging/relay/Dockerfile -t chummer-relay .
# Multi-arch (what the release workflow publishes to ghcr.io):
#   docker buildx build --platform linux/amd64,linux/arm64 -f packaging/relay/Dockerfile .
# The compile stage runs on the build machine's own architecture and
# cross-compiles for the target one, so an arm64 image needs no emulated
# (and very slow) Rust build.
FROM --platform=$BUILDPLATFORM rust:1-bookworm AS build
ARG TARGETARCH
WORKDIR /src
RUN case "${TARGETARCH:-amd64}" in \
      amd64) echo x86_64-unknown-linux-gnu > /rust-target ;; \
      arm64) echo aarch64-unknown-linux-gnu > /rust-target ;; \
      *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && if [ "$(cat /rust-target)" = aarch64-unknown-linux-gnu ] && [ "$(uname -m)" != aarch64 ]; then \
      apt-get update \
      && apt-get install -y --no-install-recommends gcc-aarch64-linux-gnu libc6-dev-arm64-cross \
      && rm -rf /var/lib/apt/lists/*; \
    fi \
 && rustup target add "$(cat /rust-target)"
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar
COPY . .
RUN t="$(cat /rust-target)" \
 && cargo build --release --locked -p chummer-relay --target "$t" \
 && cp "target/$t/release/chummer-relay" /usr/local/bin/chummer-relay

FROM debian:bookworm-slim
LABEL org.opencontainers.image.source="https://github.com/shamblashini/chummer-rs" \
      org.opencontainers.image.description="chummer-relay: relay and mailbox server for chummer-rs online campaigns" \
      org.opencontainers.image.licenses="GPL-3.0-or-later"
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates libcap2-bin \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --home /var/lib/chummer-relay --create-home chummer-relay
COPY --from=build /usr/local/bin/chummer-relay /usr/local/bin/chummer-relay
# Ports 80 and 443 as a non-root user, also with host networking (where the
# host's unprivileged-port limit applies).
RUN setcap 'cap_net_bind_service=+ep' /usr/local/bin/chummer-relay
COPY packaging/relay/relay.example.toml /etc/chummer-relay/relay.toml
USER chummer-relay
VOLUME /var/lib/chummer-relay
EXPOSE 80/tcp 443/tcp 7842/udp 7843/udp
ENTRYPOINT ["/usr/local/bin/chummer-relay", "--config", "/etc/chummer-relay/relay.toml"]
