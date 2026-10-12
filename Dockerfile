# syntax=docker/dockerfile:1.27@sha256:4edf897a3ffa55b89f906fc8cc78afdb3f1834cc9c7083565e611a8a7d5fe99e
# Stages: dev is the OpenShell environment and check is the single gate. This repository
# ships crates, not a service, so it has no build or runtime stage.

FROM rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181 AS dev
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates clang cmake git pkg-config procps \
    && rm -rf /var/lib/apt/lists/* /var/log/apt /var/log/dpkg.log \
    && rustup component add clippy rustfmt
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo install --locked \
        cargo-nextest@0.9.148 \
        cargo-deny@0.20.2 \
        cargo-hack@0.6.45 \
        cargo-semver-checks@0.51.0 \
        cargo-insta@1.49.0
COPY env/profile.d/rust.sh /etc/profile.d/rust.sh
# A development environment installs toolchains, components and crates, so the sandbox user owns them.
RUN useradd --create-home --uid 1000 --shell /bin/bash sandbox \
    && chown -R sandbox:sandbox /usr/local/rustup /usr/local/cargo
USER sandbox
LABEL org.opencontainers.image.title="veoveo-core" \
      org.opencontainers.image.source="https://github.com/VeoveoAI/veoveo-core" \
      org.opencontainers.image.licenses="Apache-2.0"

FROM dev AS source
WORKDIR /home/sandbox/src
COPY --chown=sandbox:sandbox . .

FROM source AS check
RUN cargo fmt --check \
    && cargo clippy --locked --all-targets --all-features -- -D warnings \
    && cargo deny --locked check \
    && cargo hack check --each-feature --no-dev-deps \
    && cargo nextest run --locked --profile ci
