"use client";

import { cn } from "@/lib/utils";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";
import { Loader2Icon } from "lucide-react";

function Spinner({ className, ...props }: React.ComponentProps<"svg">) {
  const { t } = useI18n();
  return (
    <Loader2Icon
      role="status"
      aria-label={t(WEBUI.common.loading)}
      className={cn("size-4 animate-spin", className)}
      {...props}
    />
  );
}

export { Spinner };
