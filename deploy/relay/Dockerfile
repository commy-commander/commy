FROM rust:1.91-slim-trixie AS build
ARG IROH_RELAY_VERSION=1.3.0
RUN cargo install iroh-relay --locked --version "=${IROH_RELAY_VERSION}" --features server --bin iroh-relay --root /out

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates wget && rm -rf /var/lib/apt/lists/* \
  && useradd --system --uid 10001 relay
COPY --from=build /out/bin/iroh-relay /usr/local/bin/iroh-relay
COPY iroh-relay.toml /etc/iroh-relay.toml
COPY --chmod=755 run-relay.sh /usr/local/bin/run-relay
USER relay
EXPOSE 3340 3478/udp 9090
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD wget -qO- http://127.0.0.1:3340/generate_204 >/dev/null 2>&1 || test -e /flags/relay-paused
CMD ["run-relay"]
