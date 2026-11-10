# CSE ingest server. Build: docker build -t cse-server:0.1.1 server/
FROM docker.io/library/rust:1.94-slim-trixie AS build
WORKDIR /src
# dependencies first, so source edits do not rebuild them
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && cargo build --release --locked && rm -rf src
COPY src ./src
RUN touch src/main.rs && cargo build --release --locked && cargo test --release --locked

FROM docker.io/library/debian:trixie-slim
RUN useradd --uid 10001 --no-create-home --home-dir /data --shell /usr/sbin/nologin cse \
    && mkdir /data && chown cse:cse /data
COPY --from=build /src/target/release/cse-server /usr/local/bin/cse-server
USER cse
VOLUME /data
EXPOSE 8080
ENV CSE_LISTEN=0.0.0.0:8080 CSE_DB=/data/cse.db CSE_TOKENS=/run/secrets/cse_tokens
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD ["cse-server", "healthcheck"]
STOPSIGNAL SIGTERM
ENTRYPOINT ["cse-server"]
