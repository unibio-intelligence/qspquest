# syntax=docker/dockerfile:1@sha256:4edf897a3ffa55b89f906fc8cc78afdb3f1834cc9c7083565e611a8a7d5fe99e
ARG RUST_IMAGE=rust:1.98.1-slim-bookworm@sha256:ff521445a372125ed4f76e1453a1f8098f2d05332d1601d30db1c1f62757e730
FROM ${RUST_IMAGE} AS builder
ARG RUST_IMAGE
ARG TARGETARCH
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends cmake clang pkg-config make ca-certificates && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY .cargo .cargo
COPY crates crates
COPY vendor vendor
COPY bindings bindings
COPY conformance conformance
COPY LICENSE ./
COPY scripts/build-mcp.sh scripts/build-mcp.sh
ARG MCP_FEATURES=sandbox,widget
RUN --mount=type=cache,id=qspquest-mcp-registry-${TARGETARCH},target=/usr/local/cargo/registry \
    --mount=type=cache,id=qspquest-mcp-git-${TARGETARCH},target=/usr/local/cargo/git \
    sh scripts/build-mcp.sh --check-features "$MCP_FEATURES" \
    && case ",$MCP_FEATURES," in *,widget,*) cargo install wasm-bindgen-cli --version 0.2.129 --locked ;; esac \
    && sh scripts/build-mcp.sh --features "$MCP_FEATURES"

RUN --mount=type=bind,source=docker/inventory.sh,target=/tmp/inventory.sh \
    sh /tmp/inventory.sh /build/inventories/builder builder "$RUST_IMAGE"

FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587 AS public
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* && useradd --uid 10001 --create-home qspquest
COPY --from=builder /build/inventories/ /opt/qspquest/inventories/
COPY --from=builder /build/target/release/qspquest-mcp /usr/local/bin/qspquest-mcp
COPY LICENSE /opt/qspquest/licenses/
COPY THIRD-PARTY-NOTICES.txt /opt/qspquest/licenses/
COPY bindings/mcp/assets/templates/iv.qsp /opt/qspquest/starter/iv.qsp
RUN --mount=type=bind,source=docker/inventory.sh,target=/tmp/inventory.sh \
    sh /tmp/inventory.sh /opt/qspquest/inventories/runtime public debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587 \
    && sha256sum /usr/local/bin/qspquest-mcp > /opt/qspquest/inventories/runtime/executable.sha256
USER 10001:10001
ENV QSPQUEST_MCP_TIER=free QSPQUEST_MCP_CONCURRENCY=2 QSPQUEST_LIBRARY_DIR=/opt/qspquest/starter
EXPOSE 8080
ENTRYPOINT ["qspquest-mcp"]
CMD ["--http", "0.0.0.0:8080"]

FROM builder AS managed-builder
RUN --mount=type=cache,id=qspquest-mcp-registry-${TARGETARCH},target=/usr/local/cargo/registry \
    --mount=type=cache,id=qspquest-mcp-git-${TARGETARCH},target=/usr/local/cargo/git \
    if ! command -v wasm-bindgen >/dev/null 2>&1; then cargo install wasm-bindgen-cli --version 0.2.129 --locked; fi \
    && sh scripts/build-mcp.sh --features sandbox,widget,managed

FROM public AS managed
USER root
COPY --from=managed-builder /build/target/release/qspquest-mcp /usr/local/bin/qspquest-mcp
# Authentication and platform URLs must be configured before this target is started.
RUN --mount=type=bind,source=docker/inventory.sh,target=/tmp/inventory.sh \
    sh /tmp/inventory.sh /opt/qspquest/inventories/runtime managed debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587 \
    && sha256sum /usr/local/bin/qspquest-mcp > /opt/qspquest/inventories/runtime/executable.sha256
USER 10001:10001
ENV QSPQUEST_MANAGED_ENABLED=1
