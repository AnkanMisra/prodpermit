FROM rust:1.97-bookworm AS builder

WORKDIR /workspace
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN cargo build --release -p recovery-api

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --create-home --uid 10001 recovery \
    && mkdir -p /data \
    && chown recovery:recovery /data

COPY --from=builder /workspace/target/release/recovery-api /usr/local/bin/recovery-api

USER recovery
ENV DATABASE_URL=sqlite:///data/recovery-control-room.db?mode=rwc
ENV PORT=8080
EXPOSE 8080

CMD ["recovery-api"]

