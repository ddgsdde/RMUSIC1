# Build Linux Core on the development host's native CPU; only runtime installation uses TARGETPLATFORM.
ARG RMUSIC_BUILD_BASE=rust:1.98-bookworm
FROM --platform=$BUILDPLATFORM node:24-bookworm-slim AS web
WORKDIR /build
COPY packages/sdk ./packages/sdk
COPY apps/web ./apps/web
WORKDIR /build/apps/web
RUN npm ci --no-audit && npm run build

FROM --platform=$BUILDPLATFORM ${RMUSIC_BUILD_BASE} AS core
ARG TARGETARCH
ENV CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2
RUN apt-get update && case "$TARGETARCH" in \
      amd64) apt-get install -y --no-install-recommends g++-x86-64-linux-gnu libc6-dev-amd64-cross ;; \
      arm64) apt-get install -y --no-install-recommends g++-aarch64-linux-gnu libc6-dev-arm64-cross ;; \
      *) exit 2 ;; \
    esac && apt-get install -y --no-install-recommends cmake pkg-config && rm -rf /var/lib/apt/lists/*
RUN case "$TARGETARCH" in amd64) rustup target add x86_64-unknown-linux-gnu ;; arm64) rustup target add aarch64-unknown-linux-gnu ;; esac
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
    CXX_x86_64_unknown_linux_gnu=x86_64-linux-gnu-g++ \
    AR_x86_64_unknown_linux_gnu=x86_64-linux-gnu-ar \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++ \
    AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY apps/desktop ./apps/desktop
COPY proto ./proto
COPY scripts/backup.py ./scripts/backup.py
# Source archives deliberately use reproducible timestamps. A reused builder can
# contain newer Cargo mtimes, so mark copied inputs fresh before dependency checks.
RUN --mount=type=cache,id=rmusic-core-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=rmusic-core-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=rmusic-core-cross-target-${TARGETARCH},target=/build/target \
    find crates apps/desktop/src-tauri/src -type f -exec touch {} + \
    && case "$TARGETARCH" in amd64) target=x86_64-unknown-linux-gnu ;; arm64) target=aarch64-unknown-linux-gnu ;; esac \
    && CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 cargo build --locked --release --target "$target" -p rmusic-core \
    && mkdir -p /out && cp "target/$target/release/rmusic-core" /out/rmusic-core

FROM python:3.13-slim-bookworm AS runtime
ARG TARGETARCH
ARG RMUSIC_SOURCE_REVISION=development
LABEL org.opencontainers.image.source="https://github.com/ddgsdde/RMUSIC1" \
      org.opencontainers.image.revision=$RMUSIC_SOURCE_REVISION \
      org.opencontainers.image.licenses="AGPL-3.0-or-later"
RUN apt-get update && apt-get install -y --no-install-recommends ffmpeg ca-certificates libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/* && useradd --uid 10001 --create-home rmusic
COPY scripts/install_fpcalc.py /app/tools/install_fpcalc.py
RUN python3 /app/tools/install_fpcalc.py
WORKDIR /app
COPY plugins/qqmusic/requirements.txt /tmp/requirements.txt
RUN pip install --no-cache-dir -r /tmp/requirements.txt \
    && mkdir -p /data /music /cache /backups && chown rmusic:rmusic /data /music /cache /backups
COPY --from=core /out/rmusic-core /usr/local/bin/rmusic-core
COPY --from=web /build/apps/web/dist /app/web
COPY plugins/qqmusic /app/plugins/qqmusic
COPY scripts/backup.py /app/tools/backup.py
COPY scripts/install_lego.py /app/tools/install_lego.py
RUN python3 /app/tools/install_lego.py --arch "$TARGETARCH" --destination /app/tools
COPY LICENSE /app/LICENSE
ENV RMUSIC_LEGO=/app/tools/lego
ENV RMUSIC_DATA_DIR=/data RMUSIC_CACHE_DIR=/cache RMUSIC_WEB_DIR=/app/web RMUSIC_PLUGIN_DIR=/data/plugins RMUSIC_BUILTIN_PLUGIN_DIR=/app/plugins RMUSIC_PYTHON=python3 RMUSIC_BIND=0.0.0.0 RMUSIC_PORT=7800 RMUSIC_CACHE_ENABLED=1
USER rmusic
EXPOSE 7800
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s CMD python3 -c "import os,urllib.request; urllib.request.urlopen('http://127.0.0.1:'+os.getenv('RMUSIC_PORT','7800')+os.getenv('RMUSIC_BASE_PATH','')+'/api/v1/status',timeout=3)"
ENTRYPOINT ["rmusic-core"]
