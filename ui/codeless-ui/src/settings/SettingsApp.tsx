import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { WindowControls } from "@/components/WindowControls";
import { IS_MAC } from "@/lib/platform";
import {
  getCrossWindowEvents,
  useShellCapabilities,
  type SettingsTab,
} from "@/lib/shell";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/modules/settings/preferences";
import {
  AiScanIcon,
  Cancel01Icon,
  InformationCircleIcon,
  Settings01Icon,
  UserMultiple02Icon,
  KeyboardIcon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { JSX, useEffect, useState } from "react";
import { AboutSection } from "./sections/AboutSection";
import { AgentsSection } from "./sections/AgentsSection";
import { GeneralSection } from "./sections/GeneralSection";
import { ModelsSection } from "./sections/ModelsSection";
import { ShortcutsSection } from "./sections/ShortcutsSection";

const TABS: { id: SettingsTab; label: string; icon: typeof Settings01Icon, component: () => JSX.Element }[] =
  [
    { id: "general", label: "General", icon: Settings01Icon, component: GeneralSection },
    { id: "shortcuts", label: "Shortcuts", icon: KeyboardIcon, component: ShortcutsSection },
    { id: "models", label: "Models", icon: AiScanIcon, component: ModelsSection },
    { id: "agents", label: "Agents", icon: UserMultiple02Icon, component: AgentsSection },
    { id: "about", label: "About", icon: InformationCircleIcon, component: AboutSection },
  ];

const VALID_TABS: SettingsTab[] = [
  "general",
  "shortcuts",
  "models",
  "agents",
  "about",
];

function readInitialTab(): SettingsTab {
  if (typeof window === "undefined") return "general";
  const url = new URL(window.location.href);
  const t = url.searchParams.get("tab");
  // Back-compat: legacy "ai" / "connections" → "models".
  if (t === "ai" || t === "connections") return "models";
  if (t && (VALID_TABS as string[]).includes(t)) return t as SettingsTab;
  return "general";
}

type InlineProps = {
  /** Provided when SettingsApp is mounted inline inside the main App
   *  shell rather than its own Tauri window. The container is full-
   *  screen but isn't *the* window, so the close affordance and tab
   *  source come from here. */
  inline?: {
    tab: SettingsTab;
    onClose: () => void;
  };
};

export function SettingsApp({ inline }: InlineProps = {}) {
  const { customWindowControls } = useShellCapabilities();
  const [active, setActive] = useState<SettingsTab>(
    inline ? inline.tab : readInitialTab,
  );
  const init = usePreferencesStore((s) => s.init);
  const ActiveSection = TABS.find(t => t.id === active)?.component;

  useEffect(() => {
    void init();
  }, [init]);

  // Sync to the latest `inline.tab` while mounted — reopening Settings
  // to a different tab while it's already on screen should switch it.
  useEffect(() => {
    if (inline) setActive(inline.tab);
  }, [inline?.tab]);

  useEffect(() => {
    const apply = (detail: string) => {
      if (detail === "ai" || detail === "connections") {
        setActive("models");
        return;
      }
      if ((VALID_TABS as string[]).includes(detail)) {
        setActive(detail as SettingsTab);
      }
    };
    const unlistenPromise = getCrossWindowEvents().listen<string>(
      "codeless:settings-tab",
      (payload) => apply(payload),
    );
    return () => {
      void unlistenPromise.then((un) => un());
    };
  }, []);

  return (
    <div
      className={cn(
        "flex flex-col overflow-hidden bg-background text-foreground select-none",
        inline ? "h-full" : "h-screen",
      )}
    >
      <header
        data-tauri-drag-region
        className={cn(
          "flex h-11 shrink-0 items-center border-b border-border/60 bg-card/60",
          inline ? "px-3" : IS_MAC ? "pr-3 pl-22" : "pr-0 pl-3",
        )}
      >
        <Tabs
          value={active}
          onValueChange={(v) => setActive(v as SettingsTab)}
          orientation="horizontal"
          className="flex-1 items-center"
          data-tauri-drag-region
        >
          <TabsList className="mx-auto h-7 bg-muted/40 px-2">
            {TABS.map((t) => (
              <TabsTrigger
                key={t.id}
                value={t.id}
                className="h-6 gap-1.5 px-2.5 text-[11.5px]"
              >
                <HugeiconsIcon icon={t.icon} size={12} strokeWidth={1.75} />
                <span>{t.label}</span>
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
        {inline ? (
          <button
            type="button"
            onClick={inline.onClose}
            aria-label="Close settings"
            title="Close (Esc)"
            className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <HugeiconsIcon icon={Cancel01Icon} size={14} strokeWidth={2} />
          </button>
        ) : customWindowControls ? (
          <WindowControls closeOnly />
        ) : null}
      </header>

      <main className="min-h-0 flex-1 overflow-y-auto px-8 pt-6 pb-7 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        <div className="mx-auto w-full max-w-160">
          {ActiveSection && <ActiveSection />}
        </div>
      </main>
    </div>
  );
}
