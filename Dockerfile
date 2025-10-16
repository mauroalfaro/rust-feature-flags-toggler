FROM rust:1-bookworm as builder
WORKDIR /app
COPY Cargo.toml .
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends libsqlite3-0 ca-certificates ^
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/rust-feature-flags-toggler /usr/local/bin/rust-feature-flags-toggler
ENV DATABASE_URL=sqlite:///data/flags.db
ENV BIND=0.0.0.0:8080
EXPOSE 8080
VOLUME ["/data"]
ENTRYPOINT ["/usr/local/bin/rust-feature-flags-toggler"]
