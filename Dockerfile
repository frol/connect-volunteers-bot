FROM ekidd/rust-musl-builder:stable AS build

COPY ./ ./
RUN cargo test --release && \
    cargo build --release && \
    mv ./target/x86_64-unknown-linux-musl/release/connect-volunteers-bot / && \
    rm -rf ./target ~/.cargo/registry ~/.cargo/git

FROM scratch

COPY --from=build /connect-volunteers-bot /
