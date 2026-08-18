import { execFileSync } from "node:child_process";
import { fileURLToPath, URL } from "node:url";
import { defineConfig, type Plugin } from "vite";
import devtools from "solid-devtools/vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";
import { VitePWA } from "vite-plugin-pwa";
import Icons from "unplugin-icons/vite";

/** The name of the build-id file within `dist/`. */
const BUILD_ID_FILE = "build-id.txt";

/**
 * Identifies this frontend build, so a running client can tell whether it came
 * from the binary currently serving it.
 *
 * The id goes two places: a `__BUILD_ID__` constant compiled into the bundle,
 * and `dist/build-id.txt`, which `rust-embed` bakes into the binary — the
 * release build does the frontend first (`cargo xtask build-release`), so the
 * embedded copy is always the one belonging to the embedded client. The server
 * reports it over `app.version` and the client compares it against its own, so
 * the two differ exactly when the client is stale. No git needed at runtime, and
 * no way for the two to drift.
 *
 * The short commit hash mirrors `radiocrate/build.rs`, which keeps a release
 * built from a clean checkout reproducible. A dirty tree — or no git at all —
 * falls back to a timestamp, which is what makes successive rebuilds
 * distinguishable while iterating.
 */
function buildId(): string {
  const git = (args: string[]): string | null => {
    try {
      return execFileSync("git", args, {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      }).trim();
    } catch {
      // No git, not a checkout, or a detached//broken repo — all the same here.
      return null;
    }
  };
  const hash = git(["rev-parse", "--short", "HEAD"]);
  if (hash === null) return `dev-${Date.now()}`;
  const dirty = git(["status", "--porcelain"]);
  return dirty === null || dirty === "" ? hash : `${hash}-dev-${Date.now()}`;
}

const BUILD_ID = buildId();

/**
 * Writes {@link BUILD_ID} to `dist/build-id.txt`.
 *
 * Emitted as a Rollup asset rather than written straight to disk so it lands
 * with the rest of the bundle — in particular *before* `vite-plugin-pwa` scans
 * `dist/` to build its precache manifest, which is why the plugin sits ahead of
 * `VitePWA` below.
 */
function buildIdFile(): Plugin {
  return {
    name: "radiocrate:build-id-file",
    apply: "build",
    generateBundle() {
      this.emitFile({
        type: "asset",
        fileName: BUILD_ID_FILE,
        source: BUILD_ID,
      });
    },
  };
}

export default defineConfig({
  plugins: [
    // Must come before `solid()`. `autoname` labels components in the
    // Solid DevTools browser extension. This plugin is a dev-only no-op in
    // production builds.
    devtools({ autoname: true }),
    solid(),
    tailwindcss(), // Tailwind v4 — no PostCSS/config file needed
    // Build-time icon inlining: each `~icons/*` import becomes a Solid SVG
    // component filled with `currentColor`. No runtime font fetch (CSP/offline safe).
    Icons({ compiler: "solid" }),
    buildIdFile(),
    VitePWA({
      strategies: "generateSW",
      registerType: "prompt", // mirror old SW: wait, don't auto-activate
      manifest: false, // keep the hand-tuned static manifest (public/)
      workbox: {
        navigateFallback: "/index.html",
        // Paths the SW must never answer from precache. `/api/` because those
        // are API calls, and `/cdn-cgi/` because that is the reserved namespace
        // an authenticating reverse proxy in front of us (Cloudflare Access)
        // serves its login and callback endpoints from. Swallowing the callback
        // is fatal and silent: the proxy authenticates the browser, redirects it
        // back to set its session cookie, the SW answers that navigation out of
        // the precache instead, the cookie is never set, and the client is left
        // permanently unable to complete a login it keeps being sent to.
        navigateFallbackDenylist: [/^\/api\//, /^\/cdn-cgi\//],
        runtimeCaching: [{ urlPattern: /^\/api\//, handler: "NetworkOnly" }],
        // The build id is read off disk by the server, never fetched by the
        // browser, so precaching it would only add a dead cache entry.
        globIgnores: [BUILD_ID_FILE],
      },
    }),
  ],
  // Compiled into the bundle so the client knows which build it is. See
  // `buildId` above.
  define: { __BUILD_ID__: JSON.stringify(BUILD_ID) },
  // The generated API client is a sibling directory (raw TS, no build step),
  // consumed through this alias. Regenerate it with `cargo xtask gen-api`.
  resolve: {
    alias: {
      "api-client": fileURLToPath(
        new URL("../api-client/src/index.ts", import.meta.url),
      ),
    },
  },
  build: { outDir: "dist", target: "esnext" },
  // In dev, Vite serves the app on its own port while the backend API runs on
  // :3000. Proxy /api so client code stays origin-relative in every environment.
  server: { proxy: { "/api": "http://localhost:3000" } },
});
