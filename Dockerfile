FROM rust:bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY ui ui
COPY schema schema
COPY fixtures fixtures
COPY examples examples
RUN cargo build --release --locked && strip /src/target/release/nexusweave

FROM debian:bookworm-slim
RUN install -d -o 10001 -g 10001 /state
COPY --from=build /src/target/release/nexusweave /usr/local/bin/nexusweave
USER 10001:10001
ENTRYPOINT ["nexusweave", "sidecar"]
CMD ["--db", "/state/evidence.sqlite"]
