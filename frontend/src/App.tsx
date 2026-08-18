import { Match, Show, Switch } from "solid-js";
import { useAppState, type Tab } from "./state/store";
import SidebarLeft from "./components/ui/SidebarLeft";
import Explorer from "./components/Explorer";
import TabBar from "./components/TabBar";
import QueryPage from "./components/QueryPage";
import ShortcutsPage from "./components/ShortcutsPage";
import NowPlaying from "./components/NowPlaying";
import CommandPalette from "./components/CommandPalette";
import UpdateBanner from "./components/UpdateBanner";
import AboutModal from "./components/AboutModal";
import SettingModal from "./components/SettingModal";

/** The content area of the active tab: the page its kind calls for, or a blank
 * panel when no tab is open. The one place tab kinds fan out into pages — every
 * other tab surface (the bar, the explorer's "Opened" list, the tab commands)
 * stays kind-agnostic. */
function TabContent() {
  const store = useAppState();
  const active = (): Tab | undefined => {
    const id = store.state.activeTabId;
    return id === null ? undefined : store.tab(id);
  };
  return (
    <Show when={active()} fallback={<div class="bg-panel min-h-0 flex-1" />}>
      {(tab) => (
        <Switch>
          <Match when={tab().kind === "query"}>
            <QueryPage tabId={tab().id} />
          </Match>
          <Match when={tab().kind === "shortcuts"}>
            <ShortcutsPage />
          </Match>
        </Switch>
      )}
    </Show>
  );
}

/** The main column, right of the explorer: tab bar, the active tab's content,
 * the client-update bar when there's one to show, and the now-playing bar
 * pinned to the bottom (which spans this column only — the explorer keeps its
 * own full height). */
function Main() {
  return (
    <div class="flex min-w-0 flex-1 flex-col">
      <TabBar />
      <TabContent />
      <UpdateBanner />
      <NowPlaying />
    </div>
  );
}

export default function App() {
  const store = useAppState();

  return (
    <div
      class="bg-panel fixed inset-0 flex"
      style={{
        "padding-top": "env(safe-area-inset-top)",
        "padding-right": "env(safe-area-inset-right)",
        "padding-bottom": "env(safe-area-inset-bottom)",
        "padding-left": "env(safe-area-inset-left)",
      }}
    >
      <SidebarLeft
        open={store.state.sidebarOpen}
        onClose={() => store.setSidebarOpen(false)}
        main={<Main />}
      >
        <Explorer />
      </SidebarLeft>

      {/* App-wide overlays, above every panel and both layouts. */}
      <CommandPalette />
      <AboutModal />
      <SettingModal />
    </div>
  );
}
