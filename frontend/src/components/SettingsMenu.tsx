import { For, type JSX } from "solid-js";
import { Icons } from "../icons";
import {
  useAppState,
  type AudioQualityPref,
  type ThemePref,
} from "../state/store";
import { SETTINGS, SETTING_KEYS } from "../state/settings";
import {
  MenuHeading,
  MenuItem,
  MenuSeparator,
  MenuToggleItem,
} from "./ui/Menu";

const THEME_OPTIONS: {
  pref: ThemePref;
  label: string;
  icon: typeof Icons.LightMode;
}[] = [
  { pref: "light", label: "Light", icon: Icons.LightMode },
  { pref: "dark", label: "Dark", icon: Icons.DarkMode },
  { pref: "system", label: "System", icon: Icons.SystemTheme },
];

const AUDIO_QUALITY_OPTIONS: {
  pref: AudioQualityPref;
  label: string;
  icon: typeof Icons.HigherQuality;
}[] = [
  { pref: "higher", label: "Higher quality", icon: Icons.HigherQuality },
  { pref: "lower", label: "Lower bandwidth", icon: Icons.LowerBandwidth },
];

/** The Settings menu body: a Light/Dark/System theme picker, a Higher
 * quality/Lower bandwidth audio streaming picker, one entry per configurable
 * setting (each opening its editor dialog), and the Keyboard shortcuts and About
 * entries. */
export default function SettingsMenu(): JSX.Element {
  const store = useAppState();
  return (
    <>
      <MenuHeading text="Theme" />
      <For each={THEME_OPTIONS}>
        {(option) => (
          <MenuToggleItem
            kind="radio"
            icon={option.icon}
            label={option.label}
            checked={store.state.theme === option.pref}
            onClick={() => store.setTheme(option.pref)}
          />
        )}
      </For>
      <MenuSeparator />
      <MenuHeading text="Audio streaming" />
      <For each={AUDIO_QUALITY_OPTIONS}>
        {(option) => (
          <MenuToggleItem
            kind="radio"
            icon={option.icon}
            label={option.label}
            checked={store.audioQuality() === option.pref}
            onClick={() => store.setAudioQuality(option.pref)}
          />
        )}
      </For>
      <MenuSeparator />
      {/* Driven by the settings catalogue, so a new setting shows up here with
          its name and needs no entry of its own. */}
      <For each={SETTING_KEYS}>
        {(key) => (
          <MenuItem
            icon={Icons.Querydown}
            label={SETTINGS[key].name}
            onClick={() => store.openSetting(key)}
          />
        )}
      </For>
      <MenuItem
        icon={Icons.Keyboard}
        label="Keyboard shortcuts"
        onClick={() => store.openShortcutsTab()}
      />
      <MenuItem
        icon={Icons.About}
        label="About RadioCrate"
        onClick={() => store.openAbout()}
      />
    </>
  );
}
