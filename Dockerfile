# Development/testing container for Querydown.
#
# Gives an isolated environment with the full toolchain (Rust workspace, wasm
# bindings, and the SvelteKit site) plus Claude Code, so the agent can run
# commands with full permissions without touching the host system.
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

# System packages + Node.js 20 (needed for the site and for Claude Code).
#
# The mesa-vulkan-drivers / libvulkan1 packages provide lavapipe, a CPU
# (software) Vulkan implementation. egui_kittest renders widget snapshots
# through wgpu, which needs a Vulkan adapter; lavapipe supplies one with no GPU
# and no display server. vulkan-tools is only for the `vulkaninfo` smoke test.
#
# python3-pil (Pillow) + python3-numpy let agents load snapshot PNGs and read
# pixels directly (`from PIL import Image`) for pixel-level measurement; the base
# image has no pip, so these come from apt. See scripts/measure_snapshot.py.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        git \
        curl \
        ca-certificates \
        pkg-config \
        cmake \
        sudo \
        mesa-vulkan-drivers \
        libvulkan1 \
        vulkan-tools \
        python3 \
        python3-pil \
        python3-numpy \
    && curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# Rust components and the wasm target/tooling used by bindings/js.
RUN rustup component add clippy rustfmt \
    && rustup target add wasm32-unknown-unknown \
    && curl -fsSL https://rustwasm.github.io/wasm-pack/installer/init.sh | sh

# Claude Code CLI.
RUN npm install -g @anthropic-ai/claude-code

# Ensure cargo is on PATH for login shells too (the base image's ENV PATH is
# otherwise reset by /etc/profile in a `bash -l` context).
RUN echo 'export PATH=/usr/local/cargo/bin:$PATH' > /etc/profile.d/cargo.sh

# Quiet X-authorization warnings during headless snapshot rendering.
#
# This is a headless container: no X/Wayland server is present, but a DISPLAY
# (e.g. ":0") can be injected at runtime — notably by the VS Code dev container.
# When it is, the Vulkan loader tries to reach that display and spams
# "Authorization required, but no authorization protocol specified" on every
# egui_kittest render, drowning out the test output. Snapshot rendering uses
# offscreen lavapipe and needs no display, so we clear DISPLAY/WAYLAND_DISPLAY.
# (This does NOT disable rendering — the lavapipe Vulkan adapter is unaffected,
# so snapshot determinism is intact.)
#
# We wire the same one-line script into every shell-entry path, because the
# warning's source depends on how a command was launched:
#   - BASH_ENV          -> non-interactive bash (the agent's tool shell, build
#                          scripts) — these source neither bashrc nor profile.
#   - /etc/bash.bashrc  -> interactive shells (the VS Code integrated terminal).
#   - /etc/profile.d    -> login shells (`bash -l`).
# `docker compose run … <cmd>` is additionally covered by entrypoint.sh, which
# unsets the vars before exec'ing the command directly.
RUN printf '%s\n' \
        '# Headless container: see the Dockerfile for why these are cleared.' \
        'unset DISPLAY WAYLAND_DISPLAY' \
        > /etc/headless-display.sh \
    && chmod 0644 /etc/headless-display.sh \
    && echo '. /etc/headless-display.sh' > /etc/profile.d/headless-display.sh \
    && echo '. /etc/headless-display.sh' >> /etc/bash.bashrc
ENV BASH_ENV=/etc/headless-display.sh

# Create the non-root user and give it ownership of the Rust toolchain dirs so
# the cargo registry/target named volumes (mounted later) initialize writable.
RUN groupadd --gid "${USER_GID}" "${USERNAME}" 2>/dev/null || true \
    && useradd --uid "${USER_UID}" --gid "${USER_GID}" -m -s /bin/bash "${USERNAME}" \
    && echo "${USERNAME} ALL=(ALL) NOPASSWD:ALL" > "/etc/sudoers.d/${USERNAME}" \
    && chmod 0440 "/etc/sudoers.d/${USERNAME}" \
    && mkdir -p /workspace /usr/local/cargo/registry /usr/local/cargo/git \
    && chown -R "${USER_UID}:${USER_GID}" /workspace /usr/local/cargo /usr/local/rustup

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
