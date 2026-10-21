import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

// No-ops when the UI runs in a plain browser (e.g. `npm run dev` without Tauri).
export async function minimizeWindow(): Promise<void> {
  if (isTauri()) await getCurrentWindow().minimize();
}

export async function closeWindow(): Promise<void> {
  if (isTauri()) await getCurrentWindow().close();
}
