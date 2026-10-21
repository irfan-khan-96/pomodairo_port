import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// Mirrors `WindowPrefs` in src-tauri/src/window_service.rs.
export type ViewMode = "mini" | "short" | "full";

export interface WindowPrefs {
  view: ViewMode;
  regular_view: ViewMode;
  width: number;
  full_height: number;
  position: [number, number] | null;
  always_on_top: boolean;
  remember_position: boolean;
}

/** Window view and options. Rust resizes the window and persists the choice. */
class ViewStore {
  prefs = $state<WindowPrefs>({
    view: "full",
    regular_view: "full",
    width: 320,
    full_height: 467,
    position: null,
    always_on_top: false,
    remember_position: true,
  });

  view = $derived(this.prefs.view);

  async connect(): Promise<UnlistenFn> {
    if (!isTauri()) return () => {};
    const unlisten = await listen<WindowPrefs>("window://prefs", (e) => (this.prefs = e.payload));
    this.prefs = await invoke<WindowPrefs>("window_prefs");
    return unlisten;
  }

  setView = (view: ViewMode) => this.call("window_set_view", { view });
  toggleMini = () => this.call("window_toggle_mini");
  setAlwaysOnTop = (enabled: boolean) => this.call("window_set_always_on_top", { enabled });
  setRememberPosition = (enabled: boolean) => this.call("window_set_remember_position", { enabled });

  private async call(cmd: string, args?: Record<string, unknown>) {
    if (!isTauri()) {
      // Browser preview: just switch the layout.
      if (cmd === "window_set_view") this.prefs.view = args!.view as ViewMode;
      if (cmd === "window_toggle_mini") this.prefs.view = this.prefs.view === "mini" ? this.prefs.regular_view : "mini";
      return;
    }
    try {
      this.prefs = await invoke<WindowPrefs>(cmd, args);
    } catch (e) {
      console.error(cmd, e);
    }
  }
}

export const view = new ViewStore();
