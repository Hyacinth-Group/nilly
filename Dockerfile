# syntax=docker/dockerfile:1
FROM rust:1-bookworm AS build
WORKDIR /src
ARG BUILD_COMMIT=
ENV BUILD_COMMIT=$BUILD_COMMIT
COPY . .
RUN cargo build --release --locked && cp target/release/discord-bot /discord-bot

FROM debian:bookworm-slim
LABEL org.opencontainers.image.source="https://github.com/krxi/discord-bot" \
      org.opencontainers.image.description="A Discord bot that grows its own personality" \
      org.opencontainers.image.licenses="MIT"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home bot \
    && mkdir -p /data/durum /data/photos /data/config && chown -R bot:bot /data
COPY --from=build /discord-bot /usr/local/bin/discord-bot
USER bot
WORKDIR /data
VOLUME ["/data/durum"]
EXPOSE 8080
ENTRYPOINT ["discord-bot"]
