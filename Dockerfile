# Development/testing container for RadioCrate.
#
# Gives an isolated environment with the full toolchain (Rust workspace, the
# SolidJS frontend's Bun/Vite/Playwright tooling, and the wasm target used by
# track-lineage) plus Claude Code, so the agent can run commands with full
# permissions without touching the host system.
#
# Pinned to the same Rust version used on the host (1.91) so build behavior
# matches. Bump this when the host toolchain changes.
FROM rust:1.91-bookworm

# Match the host user so files created in the bind-mounted workspace stay
# owned by you rather than root. Override at build time if your UID/GID differ:
#   docker compose build --build-arg USER_UID=$(id -u) --build-arg USER_GID=$(id -g)
ARG USERNAME=dev
ARG USER_UID=1000
ARG USER_GID=1000

# System packages + Node.js 20 (for Claude Code and Playwright's CLI; the
# SolidJS frontend itself runs on Bun, installed further below).
#
# ffmpeg is here for sample-data/audio/generator, which synthesizes speech to
# WAV and shells out to ffmpeg to convert it. The realistic generator next to it
# needs no such thing — it encodes FLAC through a Python wheel.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        git \
        curl \
        ca-certificates \
        ffmpeg \
        pkg-config \
        cmake \
        sudo \
        unzip \
    && curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# Rust components and the wasm target/tooling used by bindings/js.
RUN rustup component add clippy rustfmt \
    && rustup target add wasm32-unknown-unknown \
    && curl -fsSL https://rustwasm.github.io/wasm-pack/installer/init.sh | sh

# Claude Code CLI.
RUN npm install -g @anthropic-ai/claude-code

# Bun — the package manager and runtime for the SolidJS frontend (frontend/).
# `cargo xtask build-release` shells out to `bun install` + `bun run build`, and
# the frontend's dev/lint/typecheck/format/test scripts all run through Bun.
# Installed to a shared prefix (rather than a user's ~/.bun) and symlinked onto
# PATH so every user and every shell — login, interactive, non-interactive —
# finds it.
ENV BUN_INSTALL=/usr/local/bun
RUN curl -fsSL https://bun.sh/install | bash \
    && ln -s "${BUN_INSTALL}/bin/bun" /usr/local/bin/bun \
    && ln -s "${BUN_INSTALL}/bin/bunx" /usr/local/bin/bunx

# Playwright's system libraries (libnss3, libnspr4, and the X/GTK libs a headless
# Chromium links against). The frontend's whole-app visual snapshot tests drive
# Chromium through Playwright; without these, the browser fails to launch with a
# "cannot open shared object file" error. `install-deps` runs apt itself, so it
# re-populates the package lists we cleaned above; we clean them again after.
#
# The browser *binary* is deliberately NOT baked in: its build must match the
# frontend's pinned @playwright/test version, so it is fetched per-checkout with
# `bunx playwright install chromium` (see DEVELOPMENT.md).
RUN npx --yes playwright install-deps chromium \
    && rm -rf /var/lib/apt/lists/*

# DuckDB CLI — version must match the DuckDB bundled in backend/Cargo.toml
# (libduckdb-sys crate version 1.10504.0 bundles DuckDB v1.5.4).
# Update this version whenever the duckdb crate in backend/Cargo.toml is bumped.
RUN DUCKDB_VERSION="v1.5.4" \
    && ARCH=$(dpkg --print-architecture) \
    && case "$ARCH" in \
         amd64)   DUCKDB_ARCH="amd64" ;; \
         arm64)   DUCKDB_ARCH="aarch64" ;; \
         *) echo "Unsupported arch: $ARCH" && exit 1 ;; \
       esac \
    && curl -fsSL "https://github.com/duckdb/duckdb/releases/download/${DUCKDB_VERSION}/duckdb_cli-linux-${DUCKDB_ARCH}.zip" \
         -o /tmp/duckdb.zip \
    && unzip /tmp/duckdb.zip duckdb -d /usr/local/bin/ \
    && rm /tmp/duckdb.zip \
    && chmod +x /usr/local/bin/duckdb

# uv — the runner for the sample-data generator scripts (see sample-data/).
# Each of those is a uv single-file script that declares its own dependencies
# and Python version inline, so uv is the only thing that has to be installed;
# it resolves the rest on first run (which therefore needs network).
#
# Installed to a shared prefix and symlinked onto PATH for the same reason Bun
# is, and pointed at a shared directory for the Python versions it manages, so
# that the interpreter is found no matter which user runs a script rather than
# being re-downloaded into each one's home. 3.12 is baked in because that is
# what the scripts pin.
ENV UV_INSTALL_DIR=/usr/local/uv/bin \
    UV_PYTHON_INSTALL_DIR=/usr/local/uv/python
RUN curl -LsSf https://astral.sh/uv/install.sh | env INSTALLER_NO_MODIFY_PATH=1 sh \
    && ln -s "${UV_INSTALL_DIR}/uv" /usr/local/bin/uv \
    && ln -s "${UV_INSTALL_DIR}/uvx" /usr/local/bin/uvx \
    && uv python install 3.12

# Ensure cargo is on PATH for login shells too (the base image's ENV PATH is
# otherwise reset by /etc/profile in a `bash -l` context).
RUN echo 'export PATH=/usr/local/cargo/bin:$PATH' > /etc/profile.d/cargo.sh

# Create the non-root user and give it ownership of the Rust toolchain dirs so
# the cargo registry/target named volumes (mounted later) initialize writable.
RUN groupadd --gid "${USER_GID}" "${USERNAME}" 2>/dev/null || true \
    && useradd --uid "${USER_UID}" --gid "${USER_GID}" -m -s /bin/bash "${USERNAME}" \
    && echo "${USERNAME} ALL=(ALL) NOPASSWD:ALL" > "/etc/sudoers.d/${USERNAME}" \
    && chmod 0440 "/etc/sudoers.d/${USERNAME}" \
    && mkdir -p /workspace /usr/local/cargo/registry /usr/local/cargo/git \
    && chown -R "${USER_UID}:${USER_GID}" /workspace /usr/local/cargo /usr/local/rustup /usr/local/uv

COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# Script to append a timestamp to the Claude notification queue.
RUN printf '#!/bin/sh\necho "$(date \047+%%Y-%%m-%%d %%H:%%M:%%S\047)" >> ~/.claude/notification-queue.txt\n' \
    > /usr/local/bin/ding \
    && chmod +x /usr/local/bin/ding

USER ${USERNAME}
WORKDIR /workspace

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["/bin/bash"]
