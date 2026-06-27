#!/bin/bash
# Named volumes (target, cargo caches) can initialize root-owned depending on
# Docker's volume-population rules, which breaks builds run as the `dev` user.
# Fix ownership of the mount points on startup, then run the requested command.
set -e
sudo chown "$(id -u):$(id -g)" \
    /workspace/target \
    /usr/local/cargo/registry \
    /usr/local/cargo/git \
    2>/dev/null || true
# Headless container: drop any injected DISPLAY/WAYLAND_DISPLAY so the Vulkan
# loader doesn't emit X-auth warnings during snapshot rendering. This covers the
# `docker compose run … <cmd>` path, where <cmd> is exec'd directly (no bashrc/
# profile). See the matching block in the Dockerfile.
unset DISPLAY WAYLAND_DISPLAY
exec "$@"
