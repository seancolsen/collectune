import type { AppVersion, DmlResult } from "api-client";
import { PRESETS_FIXTURE, QUERIES_FIXTURE } from "../fixtures";

// The harness's stand-in backend: a `fetch` patch answering the two endpoints
// the app talks to, from the canned fixtures. It's installed by the harness
// entry alone — nothing in `main.tsx`'s import graph reaches this file — so the
// real app is untouched and the visual specs need no route mocking of their own.

/** One DML operation as the record form sends it (the fields the echo below
 * reads back). */
interface DmlOp {
  operation: "insert" | "update" | "delete";
  id: string;
  table: string;
  where?: Record<string, unknown>;
  values?: Record<string, unknown>;
}

/** The build the stub server claims to be — a fixed answer for `app.version`,
 * matching the ids the About story pins (`stories.tsx`). */
export const STUB_VERSION: AppVersion = {
  buildId: "9f21c0e",
  serverVersion: "0.4.2 (9f21c0e)",
};

/** How the stub answers a `dml` call for the story being rendered: by echoing
 * the write back as the database would, or by failing with this message. */
let dmlError: string | null = null;

/** Make every subsequent save fail with `message` — what the save-error story
 * needs, and the one backend answer a story gets to choose. */
export function failDml(message: string): void {
  dmlError = message;
}

/** The row an operation returns: what it wrote, plus the identity it wrote it
 * under (an insert's id is the database's to issue). */
function echoDml(operations: DmlOp[]): DmlResult {
  return Object.fromEntries(
    operations
      .filter((op) => op.operation !== "delete")
      .map((op) => [
        op.id,
        { id: `saved-${op.id}`, ...op.where, ...op.values },
      ]),
  ) as DmlResult;
}

/** The result for one JSON-RPC method, or a thrown message for a failed call. */
function rpcResult(method: string, params: unknown): unknown {
  switch (method) {
    case "query.list":
      return QUERIES_FIXTURE;
    case "preset.list":
      return PRESETS_FIXTURE;
    // No persisted overrides, so every command shows its built-in default and
    // every setting its built-in value.
    case "keybinding.list":
    case "setting.list":
      return [];
    // Constant, never the real `__BUILD_ID__`: a story that showed the build
    // this checkout happens to be would rewrite its baseline on every commit.
    case "app.version":
      return STUB_VERSION;
    case "dml": {
      if (dmlError !== null) throw new Error(dmlError);
      return echoDml((params as { operations: DmlOp[] }).operations);
    }
    // Every write (`query.rename`, `keybinding.set`, …) succeeds and is
    // forgotten: the harness renders one state, it doesn't persist one.
    default:
      return null;
  }
}

/** Points the app's two endpoints at the fixtures for the rest of the page's
 * life. `/api/query` — the SQL runner behind introspection and every compiled
 * query — answers empty: stories seed the results they show directly, and a
 * story that needs a schema installs one through the store. */
export function installMockApi(): void {
  const real = window.fetch.bind(window);
  window.fetch = async (input, init) => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof Request
          ? input.url
          : input.href;
    if (url.includes("/api/rpc")) {
      const body = JSON.parse(String(init?.body)) as {
        method: string;
        params: unknown;
        id: number;
      };
      let envelope: unknown;
      try {
        envelope = {
          jsonrpc: "2.0",
          result: rpcResult(body.method, body.params),
          id: body.id,
        };
      } catch (err) {
        envelope = {
          jsonrpc: "2.0",
          error: { code: 1, message: (err as Error).message },
          id: body.id,
        };
      }
      return new Response(JSON.stringify(envelope), {
        headers: { "content-type": "application/json" },
      });
    }
    if (url.includes("/api/query")) return new Response("");
    return real(input, init);
  };
}
