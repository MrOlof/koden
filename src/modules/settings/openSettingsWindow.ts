import { invoke } from "@tauri-apps/api/core";

export type SettingsTab =
  | "general"
  | "themes"
  | "terminal"
  | "shortcuts"
  | "models"
  // `agents` is the AI tab (labelled "AI"); id kept for the persisted / deep-link
  // contract. `brain` is the Koden Brain (context/memory) tab.
  | "agents"
  | "brain"
  | "about";

export async function openSettingsWindow(tab?: SettingsTab): Promise<void> {
  await invoke("open_settings_window", { tab: tab ?? null });
}
