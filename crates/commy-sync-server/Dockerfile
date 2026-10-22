FROM rust:1.97-slim-trixie AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p commy-sync-server --bin commy-sync \
    && install -D target/release/commy-sync /out/commy-sync

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends wget && rm -rf /var/lib/apt/lists/* \
  && useradd --system --uid 10002 commy
COPY --from=build /out/commy-sync /usr/local/bin/commy-sync
USER commy
ENV PORT=3002
EXPOSE 3002
HEALTHCHECK --interval=15s --timeout=5s --start-period=20s --retries=5 \
  CMD wget -qO- http://127.0.0.1:3002/healthz || exit 1
CMD ["commy-sync"]
