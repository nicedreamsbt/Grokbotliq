FROM rust:1.85-bookworm AS build
WORKDIR /app
COPY . .
RUN cargo build --release -p liquidator -p shadow -p replay -p bench

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/liquidator /usr/local/bin/
COPY --from=build /app/target/release/shadow /usr/local/bin/
COPY --from=build /app/target/release/replay /usr/local/bin/
COPY --from=build /app/config /config
ENV LIQ_CONFIG=/config/example.toml
ENV DRY_RUN=true
CMD ["liquidator"]
