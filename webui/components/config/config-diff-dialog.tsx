"use client";

import { DiffEditor, type DiffOnMount } from "@monaco-editor/react";
import { useTheme } from "next-themes";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";
import { registerOxiDnsNextYamlLanguage } from "@/lib/oxidns-next-yaml-monaco";

type MonacoApi = Parameters<DiffOnMount>[1];

interface ConfigDiffDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Left side — the selected historical snapshot. */
  original: string;
  /** Right side — the current editor content. */
  modified: string;
  originalTitle?: string;
  modifiedTitle?: string;
}

export function ConfigDiffDialog({
  open,
  onOpenChange,
  original,
  modified,
  originalTitle,
  modifiedTitle,
}: ConfigDiffDialogProps) {
  const { t } = useI18n();
  const { resolvedTheme } = useTheme();
  const theme =
    resolvedTheme === "light" ? "oxidns-next-yaml-light" : "oxidns-next-yaml-dark";
  const displayOriginalTitle =
    originalTitle ?? t(WEBUI.configEditor.historySnapshot);
  const displayModifiedTitle =
    modifiedTitle ?? t(WEBUI.configEditor.currentEditor);

  const handleBeforeMount = (monaco: MonacoApi) => {
    registerOxiDnsNextYamlLanguage(monaco);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[min(80vh,720px)] max-w-[min(1100px,calc(100vw-3rem))] flex-col gap-3 sm:max-w-[min(1100px,calc(100vw-3rem))]">
        <DialogHeader>
          <DialogTitle>{t(WEBUI.configEditor.diffTitle)}</DialogTitle>
          <DialogDescription>
            {t(WEBUI.configEditor.diffDescription, {
              original: displayOriginalTitle,
              modified: displayModifiedTitle,
            })}
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-hidden rounded-md border bg-muted/30 font-mono text-sm">
          <DiffEditor
            height="100%"
            language="yaml"
            theme={theme}
            original={original}
            modified={modified}
            beforeMount={handleBeforeMount}
            options={{
              readOnly: true,
              renderSideBySide: true,
              automaticLayout: true,
              minimap: { enabled: false },
              scrollBeyondLastLine: false,
              wordWrap: "on",
              fontSize: 13,
              lineHeight: 22,
              fontFamily:
                "JetBrains Mono, ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace",
              renderOverviewRuler: false,
            }}
          />
        </div>
      </DialogContent>
    </Dialog>
  );
}
