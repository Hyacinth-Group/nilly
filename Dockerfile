# syntax=docker/dockerfile:1
FROM rust:1-bookworm AS build
WORKDIR /src
ARG BUILD_COMMIT=
ENV BUILD_COMMIT=$BUILD_COMMIT
COPY . .
RUN cargo build --release --locked && cp target/release/nibby /nibby

FROM debian:bookworm-slim
LABEL org.opencontainers.image.source="https://github.com/krxi/nibby" \
      org.opencontainers.image.description="A self-hosted AI group-chat companion for Discord, Telegram, Slack and Matrix" \
      org.opencontainers.image.licenses="MIT"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home bot \
    && mkdir -p /data/durum /data/photos /data/config && chown -R 10001:10001 /data
COPY --from=build /nibby /usr/local/bin/nibby
USER bot
WORKDIR /data
VOLUME ["/data/durum"]
EXPOSE 8787
ENTRYPOINT ["nibby"]
