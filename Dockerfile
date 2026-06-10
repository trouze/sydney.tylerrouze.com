FROM rust:1.82-slim AS builder
RUN apt-get update && apt-get install -y pkg-config && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/wedding-rsvp .
RUN mkdir -p static data
EXPOSE 8080
ENV DATABASE_URL=sqlite:data/wedding.db
ENV RUST_LOG=wedding_rsvp=info,tower_http=info
CMD ["./wedding-rsvp"]
