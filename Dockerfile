# The backend, for Render's Docker runtime (#10).
#
# Multi-stage: a Rust toolchain to build, a slim Debian to run. The runtime image
# carries the binary and TLS roots and nothing else — no cargo, no source.
#
# Rust 1.94 matches the toolchain rainix pins for CI, so a build that passes CI
# builds here. Bump both together or the two disagree silently.
FROM rust:1.94-slim-bookworm AS build

# `libsql` links against the system TLS stack, and `reqwest` uses rustls but
# still needs certs at runtime (installed in the runtime stage below).
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Dependencies first, against a stub, so a source-only change reuses this layer.
# Render's 500 build-minutes/month are shared across the workspace, and a cold
# Rust build spends most of its time on dependencies — the frontend and every
# other deploy pay for anything wasted here.
#
# The root Cargo.toml is a workspace, and cargo loads *every* member's manifest
# before it will build any target — so each member listed there must have its
# Cargo.toml (and a stub source) present here, even one the backend does not
# depend on. A member added to the workspace but missed here fails only the
# Docker build, which CI does not run, so it goes red on Render alone (this
# happened when recipe-walk was added). Keep this list matching Cargo.toml's
# `members`.
COPY Cargo.toml Cargo.lock ./
COPY crates/recipe-core/Cargo.toml crates/recipe-core/
COPY crates/recipe-walk/Cargo.toml crates/recipe-walk/
COPY backend/Cargo.toml backend/
RUN mkdir -p crates/recipe-core/src crates/recipe-walk/src backend/src \
    && echo 'fn main() {}' > backend/src/main.rs \
    && echo '' > crates/recipe-core/src/lib.rs \
    && echo '' > crates/recipe-walk/src/lib.rs \
    && cargo build --release --bin recipe-backend \
    && rm -rf backend/src crates/recipe-core/src crates/recipe-walk/src

COPY crates crates
COPY backend backend

# Touch every real source file so cargo rebuilds from it rather than trusting a
# stub's mtime — without this a crate can silently stay the empty stub, and the
# backend then fails to find its symbols (or, worse, links the stub). Touching all
# `.rs` rather than a hand-listed few means a newly added crate the backend depends
# on cannot be forgotten here.
RUN find crates backend -name '*.rs' -exec touch {} + \
    && cargo build --release --bin recipe-backend

FROM debian:bookworm-slim AS runtime

# TLS roots: the backend fetches TheMealDB and calls the Telegram Bot API over
# https, and without these every outbound request fails certificate validation.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Not root. The process only needs to read its own binary and talk to the network.
RUN useradd --system --create-home --shell /usr/sbin/nologin recipes
USER recipes
WORKDIR /home/recipes

COPY --from=build /app/target/release/recipe-backend /usr/local/bin/recipe-backend

# Render sets PORT and routes to it; BIND_ADDR is what the binary reads. Bind
# 0.0.0.0, not localhost, or the platform cannot reach the process at all.
ENV BIND_ADDR=0.0.0.0:8080
EXPOSE 8080

# Migrations run at startup inside the binary, so there is no separate release
# step to forget. Auth config is validated at startup too: with auth mandatory, a
# backend that cannot mint a login is one that can serve nothing, so it refuses
# to boot rather than 500 on the first request.
CMD ["recipe-backend"]
