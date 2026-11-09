# syntax=docker/dockerfile:1@sha256:4edf897a3ffa55b89f906fc8cc78afdb3f1834cc9c7083565e611a8a7d5fe99e
ARG RUST_IMAGE=rust:1.98.1-slim-bookworm@sha256:ff521445a372125ed4f76e1453a1f8098f2d05332d1601d30db1c1f62757e730
ARG PYTHON_IMAGE=python:3.14.6-slim-bookworm@sha256:4c92ffcde4dd6f1ff72a24518f49fd4990b27134987dfa31a733badde66df9f8

FROM ${RUST_IMAGE} AS builder
ARG RUST_IMAGE
ARG TARGETARCH
ARG CARGO_BUILD_JOBS=4
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends python3-dev python3-venv build-essential pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && python3 -m venv /opt/build-env \
    && /opt/build-env/bin/pip install --no-cache-dir maturin==1.15.0
COPY Cargo.toml Cargo.lock pyproject.toml ./
COPY .cargo/ .cargo/
COPY crates/ crates/
COPY bindings/ bindings/
COPY LICENSE THIRD-PARTY-NOTICES.txt CITATION.cff CONTRIBUTING.md CHANGELOG.md README.md ./
COPY docker/build_info.py /tmp/build_info.py
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,id=qspquest-target-${TARGETARCH},target=/build/target \
    /opt/build-env/bin/maturin build --release --locked --compatibility linux --out /out --interpreter /opt/build-env/bin/python \
    && cargo build --release --locked -p qspquest-cli \
    && cp target/release/qspquest /out/qspquest \
    && python3 /tmp/build_info.py

RUN --mount=type=bind,source=docker/inventory.sh,target=/tmp/inventory.sh \
    INVENTORY_PYTHON=/opt/build-env/bin/python sh /tmp/inventory.sh /out/inventories/builder builder "$RUST_IMAGE"

FROM scratch AS tutorial-assets
COPY LICENSE /tutorial/LICENSE
COPY examples/synthetic/ /tutorial/examples/synthetic/
COPY docs/tutorials/synthetic.ipynb docs/tutorials/oral-pk-analysis.ipynb docs/tutorials/oral-pk-analysis.md /tutorial/docs/tutorials/
COPY docs/examples/oral_pk_analysis.py /tutorial/docs/examples/oral_pk_analysis.py
COPY docs/examples/interchange_roundtrip.py /tutorial/docs/examples/interchange_roundtrip.py
COPY docs/examples/r_quickstart.R /tutorial/docs/examples/r_quickstart.R

FROM ${PYTHON_IMAGE} AS runtime-base
ARG RUST_IMAGE
ARG PYTHON_IMAGE
ENV PYTHONUNBUFFERED=1 PYTHONDONTWRITEBYTECODE=1 PIP_DISABLE_PIP_VERSION_CHECK=1 PIP_NO_CACHE_DIR=1 \
    MPLCONFIGDIR=/tmp/matplotlib
# libSBML wheels link against Expat on minimal Debian runtimes.
RUN apt-get update && apt-get install -y --no-install-recommends libexpat1 \
    && rm -rf /var/lib/apt/lists/*
LABEL org.opencontainers.image.title="QSPQuest" \
      org.opencontainers.image.vendor="UniBio Intelligence" \
      org.opencontainers.image.version="0.1.1" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.description="QSPQuest CLI and Python analysis; bounded local scientific computation"
COPY --from=builder /out/inventories/ /opt/qspquest/inventories/
COPY --from=builder /out/qspquest /usr/local/bin/qspquest
COPY --from=builder /out/build-info.json /out/rust-licenses/ /opt/qspquest/
COPY LICENSE CITATION.cff /opt/qspquest/licenses/engine/
RUN --mount=type=bind,from=builder,source=/out,target=/wheels \
    --mount=type=bind,source=scripts/install_wheel.py,target=/tmp/install_wheel.py \
    --mount=type=bind,source=docker/constraints.txt,target=/tmp/constraints.txt \
    PIP_CONSTRAINT=/tmp/constraints.txt python /tmp/install_wheel.py /wheels --extra interchange \
    && python -m pip check \
    && python -m pip freeze --all > /opt/qspquest/requirements.freeze.txt \
    && (qspquest-doctor --analysis --interchange --json > /opt/qspquest/installation-check.json || (cat /opt/qspquest/installation-check.json && exit 1)) \
    && useradd --uid 10001 --create-home --shell /usr/sbin/nologin qspquest \
    && mkdir /work && chown 10001:10001 /work
RUN --mount=type=bind,source=docker/inventory.sh,target=/tmp/inventory.sh \
    INVENTORY_PYTHON=python sh /tmp/inventory.sh /opt/qspquest/inventories/runtime runtime "$PYTHON_IMAGE" "$RUST_IMAGE"
WORKDIR /work
USER 10001:10001
CMD ["qspquest-doctor", "--analysis"]

FROM runtime-base AS notebook
USER root
LABEL org.opencontainers.image.description="QSPQuest notebook environment with Ubi synthetic Apache-2.0 teaching assets" \
      org.opencontainers.image.licenses="Apache-2.0"
RUN --mount=type=bind,from=builder,source=/out,target=/wheels \
    --mount=type=bind,source=scripts/install_wheel.py,target=/tmp/install_wheel.py \
    --mount=type=bind,source=docker/constraints.txt,target=/tmp/constraints.txt \
    PIP_CONSTRAINT=/tmp/constraints.txt python /tmp/install_wheel.py /wheels --extra notebook,interchange \
    && python -m pip check \
    && python -m pip freeze --all > /opt/qspquest/requirements.freeze.txt
COPY --from=tutorial-assets /tutorial/ /opt/qspquest/tutorial/
COPY docker/start_notebook.py /opt/qspquest/start_notebook.py
ENV JUPYTER_CONFIG_DIR=/tmp/jupyter/config JUPYTER_DATA_DIR=/tmp/jupyter/data \
    JUPYTER_RUNTIME_DIR=/tmp/jupyter/runtime IPYTHONDIR=/tmp/ipython XDG_CACHE_HOME=/tmp/cache
EXPOSE 8888
RUN --mount=type=bind,source=docker/inventory.sh,target=/tmp/inventory.sh \
    INVENTORY_PYTHON=python sh /tmp/inventory.sh /opt/qspquest/inventories/runtime notebook "$PYTHON_IMAGE" "$RUST_IMAGE"
USER 10001:10001
CMD ["python", "/opt/qspquest/start_notebook.py"]

FROM builder AS r-builder
RUN apt-get update && apt-get install -y --no-install-recommends r-base-dev r-cran-jsonlite \
    && rm -rf /var/lib/apt/lists/*
COPY scripts/prepare-r-source.py /build/scripts/prepare-r-source.py
RUN python3 scripts/prepare-r-source.py /tmp/qspquest-r-source \
    && mkdir -p /out/r-library \
    && R CMD INSTALL --library=/out/r-library /tmp/qspquest-r-source/qspquest_0.1.1.tar.gz

RUN --mount=type=bind,source=docker/inventory.sh,target=/tmp/inventory.sh \
    INVENTORY_PYTHON=/opt/build-env/bin/python sh /tmp/inventory.sh /out/inventories/r-builder r-builder "$RUST_IMAGE"

FROM runtime-base AS r
USER root
RUN apt-get update && apt-get install -y --no-install-recommends r-base-core r-cran-jsonlite \
    && rm -rf /var/lib/apt/lists/*
COPY --from=r-builder /out/r-library/ /opt/qspquest/r-library/
COPY --from=tutorial-assets /tutorial/docs/examples/r_quickstart.R /opt/qspquest/r_quickstart.R
COPY --from=r-builder /out/inventories/r-builder/ /opt/qspquest/inventories/r-builder/
ENV R_LIBS_USER=/opt/qspquest/r-library
RUN --mount=type=bind,source=docker/inventory.sh,target=/tmp/inventory.sh \
    INVENTORY_PYTHON=python sh /tmp/inventory.sh /opt/qspquest/inventories/runtime r "$PYTHON_IMAGE" "$RUST_IMAGE"
USER 10001:10001
CMD ["Rscript", "-e", "library(qspquest); print(packageVersion('qspquest'))"]

# Same build inputs and features as the public MCP image in bindings/mcp/Dockerfile.
FROM ${RUST_IMAGE} AS mcp-builder
ARG RUST_IMAGE
ARG TARGETARCH
ARG CARGO_BUILD_JOBS=4
ARG MCP_FEATURES=sandbox,widget
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends cmake clang pkg-config make ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY .cargo/ .cargo/
COPY crates/ crates/
COPY bindings/ bindings/
COPY LICENSE SECURITY.md ./
# The server embeds its documentation from these canonical sources.
COPY spec/ spec/
COPY docs/ docs/
COPY conformance/ conformance/
COPY scripts/build-mcp.sh scripts/build-mcp.sh
RUN --mount=type=cache,id=qspquest-mcp-registry-${TARGETARCH},target=/usr/local/cargo/registry \
    --mount=type=cache,id=qspquest-mcp-git-${TARGETARCH},target=/usr/local/cargo/git \
    sh scripts/build-mcp.sh --check-features "$MCP_FEATURES" \
    && case ",$MCP_FEATURES," in *,widget,*) cargo install wasm-bindgen-cli --version 0.2.129 --locked ;; esac \
    && sh scripts/build-mcp.sh --features "$MCP_FEATURES" \
    && mkdir -p /out \
    && cp target/release/qspquest-mcp /out/qspquest-mcp \
    && printf 'qspquest-mcp %s (features: %s)\n' \
        "$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)" "$MCP_FEATURES" > /out/mcp-version.txt

RUN --mount=type=bind,source=docker/inventory.sh,target=/tmp/inventory.sh \
    sh /tmp/inventory.sh /out/inventories/mcp-builder mcp-builder "$RUST_IMAGE"

# All-in-one image: CLI, Python analysis/interchange/notebook, native R package and MCP server.
FROM notebook AS full
USER root
LABEL org.opencontainers.image.title="QSPQuest" \
      org.opencontainers.image.source="https://github.com/unibio-intelligence/qspquest" \
      org.opencontainers.image.description="QSPQuest CLI, Python, Jupyter, R and MCP server; bounded local scientific computation"
RUN apt-get update && apt-get install -y --no-install-recommends r-base-core r-cran-jsonlite \
    && rm -rf /var/lib/apt/lists/*
COPY --from=r-builder /out/r-library/ /opt/qspquest/r-library/
COPY --from=r-builder /out/inventories/r-builder/ /opt/qspquest/inventories/r-builder/
COPY --from=tutorial-assets /tutorial/docs/examples/r_quickstart.R /opt/qspquest/r_quickstart.R
COPY --from=mcp-builder /out/qspquest-mcp /usr/local/bin/qspquest-mcp
COPY --from=mcp-builder /out/mcp-version.txt /opt/qspquest/mcp-version.txt
COPY --from=mcp-builder /out/inventories/mcp-builder/ /opt/qspquest/inventories/mcp-builder/
COPY bindings/mcp/assets/templates/iv.qsp /opt/qspquest/starter/iv.qsp
COPY docker/versions.py /opt/qspquest/versions.py
ENV R_LIBS_USER=/opt/qspquest/r-library \
    QSPQUEST_MCP_TIER=free QSPQUEST_MCP_CONCURRENCY=2 QSPQUEST_LIBRARY_DIR=/opt/qspquest/starter
EXPOSE 8080 8888
RUN --mount=type=bind,source=docker/inventory.sh,target=/tmp/inventory.sh \
    INVENTORY_PYTHON=python sh /tmp/inventory.sh /opt/qspquest/inventories/runtime full "$PYTHON_IMAGE" "$RUST_IMAGE"
USER 10001:10001
CMD ["python", "/opt/qspquest/versions.py"]

# A plain build produces the smaller batch/CLI image.
FROM runtime-base AS runtime
