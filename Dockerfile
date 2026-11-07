FROM rust:1.95-bookworm AS build
WORKDIR /app
ENV CARGO_PROFILE_RELEASE_OPT_LEVEL=3
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --locked --release -p vietqr-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/vietqr-server /usr/local/bin/vietqr-server
USER 65532:65532
ENV BIND_ADDR=0.0.0.0:3000
EXPOSE 3000
ENTRYPOINT ["vietqr-server"]
