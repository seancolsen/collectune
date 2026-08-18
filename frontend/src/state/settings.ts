import type { Setting } from "api-client";
import { DEFAULT_PRELUDE } from "../query/definition";

// The catalogue of user-configurable settings: every key the app knows, what it
// defaults to when the user hasn't customized it, and how to describe it in the
// UI.
//
// The list is static and lives here, in the frontend, alone. The backend's
// `settings.settings` table (migration 0004) is a bare key/value store that
// holds *only* what the user has changed — a key with no row simply has its
// default from this file. Nothing on the server interprets a key or a value, so
// adding a setting is a change to this file plus whatever reads it.

/** What the app knows about one setting, apart from its current value. */
export interface SettingDefinition {
  /** Short human-readable name — the Settings menu entry and the dialog title. */
  name: string;
  /** A sentence explaining what the setting does, shown in its editor. */
  description: string;
  /** The value in force when the user hasn't customized it (no stored row). */
  default: string;
}

/** Every setting, keyed by the string stored in `settings.settings."key"`. */
export const SETTINGS = {
  querydown_prelude: {
    name: "Querydown Prelude",
    description:
      "This Querydown code will prepend all queries for the purpose of defining share variables and functions.",
    default: DEFAULT_PRELUDE,
  },
} satisfies Record<string, SettingDefinition>;

/** A key of {@link SETTINGS} — the app's whole settings key space, statically. */
export type SettingKey = keyof typeof SETTINGS;

/** Every settings key, in the order the Settings menu lists them. */
export const SETTING_KEYS = Object.keys(SETTINGS) as SettingKey[];

/** The settings the user has customized. A missing key means "use the default",
 * which is the same thing the absence of a row in `settings.settings` means. */
export type SettingOverrides = Partial<Record<SettingKey, string>>;

export function isSettingKey(key: string): key is SettingKey {
  return Object.hasOwn(SETTINGS, key);
}

/** Reads `setting.list` into overrides, dropping keys this build doesn't know.
 * An unknown key is another build's setting, not corruption: it stays in the
 * table untouched, so switching back and forth doesn't lose it. */
export function overridesFromEntries(
  entries: readonly Setting[],
): SettingOverrides {
  const overrides: SettingOverrides = {};
  for (const entry of entries) {
    if (isSettingKey(entry.key)) overrides[entry.key] = entry.value;
  }
  return overrides;
}

/** A setting's value in force: the user's, or the built-in default. */
export function settingValue(
  overrides: SettingOverrides,
  key: SettingKey,
): string {
  return overrides[key] ?? SETTINGS[key].default;
}

/** The overrides after setting `key` to `value` — with a value equal to the
 * default recorded as *no* override, so "customized" stays the same question as
 * "differs from the default" and the stored row disappears (see
 * `store.saveSetting`). */
export function withSetting(
  overrides: SettingOverrides,
  key: SettingKey,
  value: string,
): SettingOverrides {
  const next = { ...overrides };
  if (value === SETTINGS[key].default) delete next[key];
  else next[key] = value;
  return next;
}
