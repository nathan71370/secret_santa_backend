FROM rust:1.96-slim-bookworm AS build
WORKDIR /app
# Couche de dépendances : recompilée seulement quand Cargo.toml / Cargo.lock changent.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && touch src/lib.rs \
    && cargo build --release && rm -rf src
COPY migrations migrations
COPY src src
RUN touch src/main.rs src/lib.rs && cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /data santa \
    && mkdir -p /data/uploads && chown -R santa /data
COPY --from=build /app/target/release/secret_santa /usr/local/bin/secret_santa
USER santa
ENV DATABASE_PATH=/data/santa.db \
    UPLOADS_DIR=/data/uploads \
    BIND_ADDR=0.0.0.0:8080 \
    TZ=Europe/Paris
EXPOSE 8080
CMD ["secret_santa"]
