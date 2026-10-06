# Build from the repository root:
#   docker build -f packaging/relay/Dockerfile -t chummer-relay .
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p chummer-relay \
 && cp target/release/chummer-relay /usr/local/bin/chummer-relay

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --home /var/lib/chummer-relay --create-home chummer-relay
COPY --from=build /usr/local/bin/chummer-relay /usr/local/bin/chummer-relay
COPY packaging/relay/relay.example.toml /etc/chummer-relay/relay.toml
USER chummer-relay
VOLUME /var/lib/chummer-relay
EXPOSE 80/tcp 443/tcp 7842/udp 7843/udp
ENTRYPOINT ["/usr/local/bin/chummer-relay", "--config", "/etc/chummer-relay/relay.toml"]
