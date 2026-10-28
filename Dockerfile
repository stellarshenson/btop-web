# syntax=docker/dockerfile:1

FROM rust:1.99.1-bookworm AS builder
WORKDIR /src
COPY . .
# cargo-chef is deferred until dependency-layer rebuild time justifies it.
RUN cargo build --locked --release --bin btop-web

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /src/target/release/btop-web /usr/local/bin/btop-web
USER nonroot:nonroot
EXPOSE 40290
ENTRYPOINT ["/usr/local/bin/btop-web"]
