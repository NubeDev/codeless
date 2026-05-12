import { cn } from "@/lib/utils";
import {
  useShellCapabilities,
  useWindowControls,
} from "@/lib/shell";
import {
  Cancel01Icon,
  Copy01Icon,
  MinusSignIcon,
  SquareIcon,
} from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useEffect, useState } from "react";

type Props = {
  /** Render only the close button (used by the settings window). */
  closeOnly?: boolean;
};

export function WindowControls({ closeOnly = false }: Props) {
  const { customWindowControls } = useShellCapabilities();
  const wc = useWindowControls();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!customWindowControls || closeOnly) return;
    let unlisten: (() => void) | undefined;
    void wc.isMaximized().then(setMaximized);
    void wc
      .onResized(() => {
        void wc.isMaximized().then(setMaximized);
      })
      .then((un) => {
        unlisten = un;
      });
    return () => unlisten?.();
  }, [customWindowControls, closeOnly, wc]);

  if (!customWindowControls) return null;

  return (
    <div className="flex h-full shrink-0 items-center gap-0.5 pr-1">
      {!closeOnly && (
        <>
          <CtlButton ariaLabel="Minimize" onClick={() => void wc.minimize()}>
            <HugeiconsIcon icon={MinusSignIcon} size={12} strokeWidth={2} />
          </CtlButton>
          <CtlButton
            ariaLabel={maximized ? "Restore" : "Maximize"}
            onClick={() => void wc.toggleMaximize()}
          >
            <HugeiconsIcon
              icon={maximized ? Copy01Icon : SquareIcon}
              size={12}
              strokeWidth={2}
            />
          </CtlButton>
        </>
      )}
      <CtlButton ariaLabel="Close" onClick={() => void wc.close()} danger>
        <HugeiconsIcon icon={Cancel01Icon} size={14} strokeWidth={2} />
      </CtlButton>
    </div>
  );
}

function CtlButton({
  ariaLabel,
  onClick,
  children,
  danger,
}: {
  ariaLabel: string;
  onClick: () => void;
  children: React.ReactNode;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={ariaLabel}
      title={ariaLabel}
      onClick={onClick}
      className={cn(
        "grid size-7 place-items-center rounded-md text-muted-foreground transition-colors",
        danger
          ? "hover:bg-destructive/15 hover:text-destructive"
          : "hover:bg-accent hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}
