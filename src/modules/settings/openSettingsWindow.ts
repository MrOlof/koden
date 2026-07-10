import { invoke } from "@tauri-apps/api/core";

export type SettingsTab =
  | "general"
  | "themes"
  | "terminal"
  | "shortcuts"
  | "models"
  // `agents` is the Librarian tab (chat persona + engine); id kept for the
  // persisted / deep-link contract. `brain` is the Koden Brain index tab.
  | "agents"
  | "brain"
  | "about";

export async function openSettingsWindow(tab?: SettingsTab): Promise<void> {
  await invoke("open_settings_window", { tab: tab ?? null });
}
