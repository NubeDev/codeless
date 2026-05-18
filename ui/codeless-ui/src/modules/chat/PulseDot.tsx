import { motion } from "motion/react";

import { cn } from "@/lib/utils";

// A small pulsating dot used as a live-indicator across the chat
// surfaces (in-flight assistant bubble, agent-activity pill, stage
// header). Lives in the chat module because the chat renderer is its
// primary consumer; the in-job lifecycle widgets in `RunPane` import
// from here too so the visual vocabulary stays consistent.
export function PulseDot({ color }: { color: string }) {
  return (
    <span className="relative inline-block h-2 w-2">
      <motion.span
        aria-hidden
        animate={{ opacity: [0.4, 1, 0.4], scale: [1, 1.2, 1] }}
        transition={{ duration: 1.4, repeat: Infinity }}
        className={cn("absolute inset-0 rounded-full", color)}
      />
    </span>
  );
}
