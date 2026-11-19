FROM rust:1.97-slim-trixie AS web
ARG COMMY_WEB_RELAYS=https://relay.commy.chat:8443,https://relay-staging.commy.chat:8443
RUN apt-get update && apt-get install -y --no-install-recommends clang && rm -rf /var/lib/apt/lists/*
COPY --from=node:24.22.1-trixie-slim@sha256:89c5244bc22a9bc3287d89ebc7e49b9cb5bb84cd40e6175ac3421d5a9f68b95a /usr/local/bin/node /usr/local/bin/node
RUN rustup target add wasm32-unknown-unknown
WORKDIR /src
COPY Cargo.lock ./
RUN cargo install wasm-bindgen-cli --locked --version "$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/"/, "", $3); print $3 }' Cargo.lock)"
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,sharing=locked \
    find /src -path /src/target -prune -o -type f -exec touch {} + \
    && COMMY_WEB_OUT=/out COMMY_WEB_BASE=/web/ COMMY_WEB_RELAYS="$COMMY_WEB_RELAYS" node apps/web/build.mjs

FROM node:24.22.1-alpine@sha256:06d7adf3e0ef61a13f1d28682726366f97a15aa1888c934d3ecec86c33918b53 AS deps
WORKDIR /app
COPY auth/package.json auth/package-lock.json ./
RUN npm ci --omit=dev --no-audit --no-fund

FROM node:24.22.1-alpine@sha256:06d7adf3e0ef61a13f1d28682726366f97a15aa1888c934d3ecec86c33918b53 AS pages
WORKDIR /app
COPY auth/package.json auth/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY auth/pages ./pages
RUN node pages/build.mjs

FROM node:24.22.1-alpine@sha256:06d7adf3e0ef61a13f1d28682726366f97a15aa1888c934d3ecec86c33918b53
WORKDIR /app
ENV NODE_ENV=production PORT=3001
COPY --from=deps /app/node_modules ./node_modules
COPY auth/package.json ./
COPY auth/src ./src
COPY auth/assets ./assets
COPY --from=pages /app/pages/dist ./pages/dist
COPY --from=web /out ./web
ARG COMMY_VERSION=dev
ENV COMMY_VERSION=${COMMY_VERSION}
USER node
EXPOSE 3001
HEALTHCHECK --interval=15s --timeout=5s --start-period=30s --retries=5 \
  CMD wget -qO- http://127.0.0.1:3001/healthz || exit 1
CMD ["node", "src/server.js"]
