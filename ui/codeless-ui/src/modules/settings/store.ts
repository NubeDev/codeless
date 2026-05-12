import { getCrossWindowEvents, getStore } from "@/lib/shell";

const STORE_NAME = "prefs";
import {
  DEFAULT_AUTOCOMPLETE_MODEL,
  DEFAULT_MODEL_ID,
  LMSTUDIO_DEFAULT_BASE_URL,
  type AutocompleteProviderId,
  type ModelId,
} from "@/modules/ai/config";
import type { KeyBinding, ShortcutId } from "@/modules/shortcuts/shortcuts";

export type ThemePref = "system" | "light" | "dark";

export const EDITOR_THEMES = [
  "atomone",
  "aura",
  "copilot",
  "github-dark",
  "github-light",
  "nord",
  "tokyo-night",
  "xcode-dark",
  "xcode-light",
] as const;

export type EditorThemeId = (typeof EDITOR_THEMES)[number];

export const EDITOR_THEME_LABELS: Record<EditorThemeId, string> = {
  atomone: "Atom One",
  aura: "Aura",
  copilot: "Copilot",
  "github-dark": "GitHub Dark",
  "github-light": "GitHub Light",
  nord: "Nord",
  "tokyo-night": "Tokyo Night",
  "xcode-dark": "Xcode Dark",
  "xcode-light": "Xcode Light",
};

export type Preferences = {
  theme: ThemePref;
  defaultModelId: ModelId;
  editorTheme: EditorThemeId;
  customInstructions: string;
  autostart: boolean;
  restoreWindowState: boolean;
  autocompleteEnabled: boolean;
  autocompleteProvider: AutocompleteProviderId;
  autocompleteModelId: string;
  lmstudioBaseURL: string;
  vimMode: boolean;
  shortcuts: Record<ShortcutId, KeyBinding[]>;
};

const KEY_THEME = "theme";
const KEY_DEFAULT_MODEL = "defaultModelId";
const KEY_EDITOR_THEME = "editorTheme";
const KEY_CUSTOM_INSTRUCTIONS = "customInstructions";
const KEY_AUTOSTART = "autostart";
const KEY_RESTORE_WINDOW = "restoreWindowState";
const KEY_AUTOCOMPLETE_ENABLED = "autocompleteEnabled";
const KEY_AUTOCOMPLETE_PROVIDER = "autocompleteProvider";
const KEY_AUTOCOMPLETE_MODEL = "autocompleteModelId";
const KEY_LMSTUDIO_BASE_URL = "lmstudioBaseURL";
const KEY_VIM_MODE = "vimMode";
const KEY_SHORTCUTS = "shortcuts";

export const DEFAULT_PREFERENCES: Preferences = {
  theme: "system",
  defaultModelId: DEFAULT_MODEL_ID,
  editorTheme: "atomone",
  customInstructions: "",
  autostart: false,
  restoreWindowState: true,
  autocompleteEnabled: false,
  autocompleteProvider: "cerebras",
  autocompleteModelId: DEFAULT_AUTOCOMPLETE_MODEL.cerebras,
  lmstudioBaseURL: LMSTUDIO_DEFAULT_BASE_URL,
  vimMode: false,
  shortcuts: {} as Record<ShortcutId, KeyBinding[]>,
};

// The persistent store (Tauri's `LazyStore` on desktop) only fires
// `onChange` within the writing process. The settings page lives in a
// separate webview on desktop, so writes there never reach the main
// window's subscribers via the store alone. Mirror every setter
// through the cross-window event bus so any window can listen. In
// browser/mobile the in-process bus collapses to a same-window callback.
const PREFS_CHANGED_EVENT = "codeless://prefs-changed";

async function writePref<T>(key: string, value: T): Promise<void> {
  await getStore(STORE_NAME).set(key, value);
  await getCrossWindowEvents().emit(PREFS_CHANGED_EVENT, { key, value });
}

export async function loadPreferences(): Promise<Preferences> {
  // Single round-trip — fetching keys individually fans out to one
  // request per setting and is the dominant boot cost on Tauri.
  const entries = await getStore(STORE_NAME).loadAll();
  const map = new Map<string, unknown>(entries);
  const get = <T>(k: string): T | undefined => map.get(k) as T | undefined;
  return {
    theme: get<ThemePref>(KEY_THEME) ?? DEFAULT_PREFERENCES.theme,
    defaultModelId:
      get<ModelId>(KEY_DEFAULT_MODEL) ?? DEFAULT_PREFERENCES.defaultModelId,
    editorTheme:
      get<EditorThemeId>(KEY_EDITOR_THEME) ?? DEFAULT_PREFERENCES.editorTheme,
    customInstructions:
      get<string>(KEY_CUSTOM_INSTRUCTIONS) ??
      DEFAULT_PREFERENCES.customInstructions,
    autostart: get<boolean>(KEY_AUTOSTART) ?? DEFAULT_PREFERENCES.autostart,
    restoreWindowState:
      get<boolean>(KEY_RESTORE_WINDOW) ??
      DEFAULT_PREFERENCES.restoreWindowState,
    autocompleteEnabled:
      get<boolean>(KEY_AUTOCOMPLETE_ENABLED) ??
      DEFAULT_PREFERENCES.autocompleteEnabled,
    autocompleteProvider:
      get<AutocompleteProviderId>(KEY_AUTOCOMPLETE_PROVIDER) ??
      DEFAULT_PREFERENCES.autocompleteProvider,
    autocompleteModelId:
      get<string>(KEY_AUTOCOMPLETE_MODEL) ??
      DEFAULT_PREFERENCES.autocompleteModelId,
    lmstudioBaseURL:
      get<string>(KEY_LMSTUDIO_BASE_URL) ?? DEFAULT_PREFERENCES.lmstudioBaseURL,
    vimMode: get<boolean>(KEY_VIM_MODE) ?? DEFAULT_PREFERENCES.vimMode,
    shortcuts:
      get<Record<ShortcutId, KeyBinding[]>>(KEY_SHORTCUTS) ??
      DEFAULT_PREFERENCES.shortcuts,
  };
}

export async function setTheme(value: ThemePref): Promise<void> {
  await writePref(KEY_THEME, value);
}

export async function setDefaultModel(value: ModelId): Promise<void> {
  await writePref(KEY_DEFAULT_MODEL, value);
}

export async function setEditorTheme(value: EditorThemeId): Promise<void> {
  await writePref(KEY_EDITOR_THEME, value);
}

export async function setCustomInstructions(value: string): Promise<void> {
  await writePref(KEY_CUSTOM_INSTRUCTIONS, value);
}

export async function setAutostart(value: boolean): Promise<void> {
  await writePref(KEY_AUTOSTART, value);
}

export async function setRestoreWindowState(value: boolean): Promise<void> {
  await writePref(KEY_RESTORE_WINDOW, value);
}

export async function setAutocompleteEnabled(value: boolean): Promise<void> {
  await writePref(KEY_AUTOCOMPLETE_ENABLED, value);
}

export async function setAutocompleteProvider(
  value: AutocompleteProviderId
): Promise<void> {
  await writePref(KEY_AUTOCOMPLETE_PROVIDER, value);
}

export async function setAutocompleteModelId(value: string): Promise<void> {
  await writePref(KEY_AUTOCOMPLETE_MODEL, value);
}

export async function setLmstudioBaseURL(value: string): Promise<void> {
  await writePref(KEY_LMSTUDIO_BASE_URL, value);
}

export async function setVimMode(value: boolean): Promise<void> {
  await writePref(KEY_VIM_MODE, value);
}

// Shortcuts intentionally skip the cross-window mirror — they're only
// ever edited from the Settings window and read by the main window on
// hydrate; a missed live-update is acceptable for a rarely-changed
// preference.
export async function setShortcuts(
  value: Record<ShortcutId, KeyBinding[]> | {}
): Promise<void> {
  await getStore(STORE_NAME).set(KEY_SHORTCUTS, value);
}

export async function resetShortcuts(): Promise<void> {
  await getStore(STORE_NAME).set(KEY_SHORTCUTS, DEFAULT_PREFERENCES.shortcuts);
}

export type PrefKey = keyof Preferences;

/** Subscribe to changes from any window (settings → main). */
export async function onPreferencesChange(
  cb: (key: PrefKey, value: unknown) => void,
): Promise<() => void> {
  const map: Record<string, PrefKey> = {
    [KEY_THEME]: "theme",
    [KEY_DEFAULT_MODEL]: "defaultModelId",
    [KEY_EDITOR_THEME]: "editorTheme",
    [KEY_CUSTOM_INSTRUCTIONS]: "customInstructions",
    [KEY_AUTOSTART]: "autostart",
    [KEY_RESTORE_WINDOW]: "restoreWindowState",
    [KEY_AUTOCOMPLETE_ENABLED]: "autocompleteEnabled",
    [KEY_AUTOCOMPLETE_PROVIDER]: "autocompleteProvider",
    [KEY_AUTOCOMPLETE_MODEL]: "autocompleteModelId",
    [KEY_LMSTUDIO_BASE_URL]: "lmstudioBaseURL",
    [KEY_VIM_MODE]: "vimMode",
    [KEY_SHORTCUTS]: "shortcuts",
  };
  // Same-process writes still fire the store's `onChange` immediately;
  // cross-window writes arrive via the event mirror emitted by writePref.
  const unsubLocal = await getStore(STORE_NAME).onChange((key, value) => {
    const mapped = map[key];
    if (mapped) cb(mapped, value);
  });
  const unsubEvent = await getCrossWindowEvents().listen<{
    key: string;
    value: unknown;
  }>(PREFS_CHANGED_EVENT, (payload) => {
    const mapped = map[payload.key];
    if (mapped) cb(mapped, payload.value);
  });
  return () => {
    unsubLocal();
    unsubEvent();
  };
}

// API key changes are stored in OS keychain (not the prefs store), so
// we broadcast through the cross-window bus for any window to react.
const KEYS_CHANGED_EVENT = "codeless://ai-keys-changed";

export async function emitKeysChanged(): Promise<void> {
  await getCrossWindowEvents().emit(KEYS_CHANGED_EVENT);
}

export function onKeysChanged(cb: () => void): Promise<() => void> {
  return getCrossWindowEvents().listen(KEYS_CHANGED_EVENT, () => cb());
}
