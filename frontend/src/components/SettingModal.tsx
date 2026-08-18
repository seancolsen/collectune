import { createSignal, onMount, Show, type JSX } from "solid-js";
import { useAppState } from "../state/store";
import { SETTINGS, type SettingKey } from "../state/settings";
import { Modal } from "./ui/Modal";

/** The editor for one setting: its name, what it does, and its value in a
 * monospace text area — the values here are code (the Querydown prelude), not
 * prose. Takes its values as props, like {@link AboutDialog}, so the visual
 * harness can render it without a backend; {@link SettingModal} is the wired
 * version.
 *
 * Edits are held in a local draft until Save, so Cancel (and Escape) leave the
 * stored value alone. "Reset to default" only fills the draft with the default —
 * saving that is what actually clears the customization, since the store records
 * a value equal to the default as no override at all. */
export function SettingDialog(props: {
  name: string;
  description: string;
  value: string;
  defaultValue: string;
  onSave: (value: string) => void;
  onClose: () => void;
}): JSX.Element {
  // The draft is `undefined` until the first edit, and the field shows the
  // stored value until then — rather than seeding a signal from `props.value`,
  // which would read it once, outside any tracking scope.
  const [edited, setEdited] = createSignal<string>();
  const draft = () => edited() ?? props.value;
  const isDefault = () => draft() === props.defaultValue;
  const changed = () => draft() !== props.value;

  // Focus the field, but at the *top* of the value: a textarea autofocused with
  // content lands the caret at the end and scrolls there, which for a prelude
  // taller than the field means opening on its last line.
  let field: HTMLTextAreaElement | undefined;
  onMount(() => {
    field?.focus();
    field?.setSelectionRange(0, 0);
    if (field) field.scrollTop = 0;
  });

  return (
    <Modal onClose={() => props.onClose()} width="640px">
      <h2 class="text-ink mb-1 text-base font-semibold">{props.name}</h2>
      <p class="text-ink-weak mb-3 text-sm">{props.description}</p>
      <textarea
        ref={(el) => (field = el)}
        spellcheck={false}
        autocapitalize="off"
        autocorrect="off"
        class="bg-sheet border-edge text-ink focus:border-accent h-64 max-h-[60vh] w-full resize-y rounded-md border p-3 font-mono text-xs leading-5 outline-none"
        value={draft()}
        onInput={(e) => setEdited(e.currentTarget.value)}
      />
      <div class="mt-4 flex flex-wrap items-center justify-end gap-2">
        <button
          type="button"
          disabled={isDefault()}
          class="text-ink hover:bg-hover mr-auto rounded-md px-3 py-1.5 text-sm disabled:opacity-40"
          onClick={() => setEdited(props.defaultValue)}
        >
          Reset to default
        </button>
        <button
          type="button"
          class="text-ink hover:bg-hover rounded-md px-3 py-1.5 text-sm"
          onClick={() => props.onClose()}
        >
          Cancel
        </button>
        <button
          type="button"
          disabled={!changed()}
          class="bg-accent text-panel rounded-md px-3 py-1.5 text-sm disabled:opacity-40"
          onClick={() => props.onSave(draft())}
        >
          Save
        </button>
      </div>
    </Modal>
  );
}

/** The setting editor wired to the store: raised by the Settings menu's entry
 * for whichever setting it names, saved back through `saveSetting` (which
 * persists it and re-runs the open queries under the new value). */
function SettingModalBody(props: { settingKey: SettingKey }): JSX.Element {
  const store = useAppState();
  const definition = () => SETTINGS[props.settingKey];

  return (
    <SettingDialog
      name={definition().name}
      description={definition().description}
      value={store.settingValue(props.settingKey)}
      defaultValue={definition().default}
      onSave={(value) => {
        store.saveSetting(props.settingKey, value);
        store.closeSetting();
      }}
      onClose={() => store.closeSetting()}
    />
  );
}

/** The setting editor's mount point: an app-wide overlay, like the About
 * dialog. */
export default function SettingModal(): JSX.Element {
  const store = useAppState();
  return (
    <Show when={store.state.settingEditor}>
      {(key) => <SettingModalBody settingKey={key()} />}
    </Show>
  );
}
