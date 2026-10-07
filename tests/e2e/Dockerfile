# Runtime image for the end-to-end tests: the release binaries built on the
# host (run.sh builds them; they need glibc 2.34+), iproute2 for tc/netem.
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates iproute2 procps \
 && rm -rf /var/lib/apt/lists/*
COPY bin/ /usr/local/bin/
ENV CHUMMER_RESOURCES=/resources RUST_LOG=info,iroh=warn,iroh_relay=warn
