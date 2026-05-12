import {
  ComputerIcon,
  Moon02Icon,
  Sun03Icon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";

import { Button } from "@/components/ui/button";
import { useTheme, type Theme } from "./ThemeProvider";

// Cycle order is light -> dark -> system so a single click round-trips
// the explicit pair quickly and the third click returns control to the
// OS preference. Stored in the same `ThemePref` that the settings
// window reads, so toggling from the header and toggling from settings
// stay in lockstep.
const NEXT: Record<Theme, Theme> = {
  light: "dark",
  dark: "system",
  system: "light",
};

const LABEL: Record<Theme, string> = {
  light: "Theme: light",
  dark: "Theme: dark",
  system: "Theme: follows system",
};

export function ThemeToggle() {
  const { theme, setTheme } = useTheme();
  const icon =
    theme === "light" ? Sun03Icon : theme === "dark" ? Moon02Icon : ComputerIcon;
  return (
    <Button
      variant="ghost"
      size="icon"
      className="size-7 shrink-0 rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
      onClick={() => setTheme(NEXT[theme])}
      title={`${LABEL[theme]} — click to switch`}
      aria-label="Toggle theme"
    >
      <HugeiconsIcon icon={icon} size={15} strokeWidth={1.75} />
    </Button>
  );
}
