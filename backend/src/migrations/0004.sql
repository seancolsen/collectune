-- User-customized application settings, as key/value pairs.
--
-- A row exists only for a setting the user has changed: absence means "use the
-- built-in default". Deleting a row is therefore how a setting is reset.
--
-- The key space, the defaults, and the meaning of each `value` all live in the
-- frontend (`frontend/src/state/settings.ts`) — the backend stores strings it
-- never interprets. That keeps a key an older or newer client doesn't know
-- about harmless: unknown keys are ignored on load rather than lost.
create table settings.settings (
  "key" text primary key,
  "value" text not null
);
