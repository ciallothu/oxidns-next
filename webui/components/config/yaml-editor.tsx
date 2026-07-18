"use client";

import Editor, { type OnMount } from "@monaco-editor/react";
import { useTheme } from "next-themes";
import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";
import type { ConfigField } from "@/lib/plugin-definitions";
import { ConfigValidationError, validateConfigText } from "@/lib/oxidns-next-api";
import {
  clearOxiDnsNextYamlModelContext,
  registerOxiDnsNextYamlLanguage,
  setOxiDnsNextYamlModelContext,
  updateOxiDnsNextYamlMarkers,
  type OxiDnsNextYamlDiagnostic,
  type OxiDnsNextYamlEditorVariant,
} from "@/lib/oxidns-next-yaml-monaco";
import type { PluginInstance } from "@/lib/types";
import { cn } from "@/lib/utils";
import { WEBUI } from "@/lib/i18n";
import { useI18n } from "@/lib/i18n/provider";

type MonacoApi = Parameters<OnMount>[1];
type MonacoEditor = Parameters<OnMount>[0];
type MonacoModel = ReturnType<MonacoEditor["getModel"]>;

export interface YamlEditorHandle {
  jumpToLine: (line: number) => void;
}

interface YamlEditorProps {
  value: string;
  onChange?: (value: string) => void;
  readOnly?: boolean;
  className?: string;
  lineNumbers?: boolean;
  variant?: OxiDnsNextYamlEditorVariant;
  plugins?: PluginInstance[];
  pluginKind?: string;
  fields?: ConfigField[];
  currentPluginName?: string;
  /** Bound to Cmd+S (macOS) / Ctrl+S (Windows/Linux) via Monaco. */
  onSave?: () => void;
  /** Run the backend /config/validate pass. Disable in offline mode. */
  backendValidation?: boolean;
}

export const YamlEditor = forwardRef<YamlEditorHandle, YamlEditorProps>(
  function YamlEditor(
    {
      value,
      onChange,
      readOnly = false,
      className,
      lineNumbers = true,
      variant = "generic",
      plugins,
      pluginKind,
      fields,
      currentPluginName,
      onSave,
      backendValidation = true,
    },
    ref,
  ) {
    const { locale, t } = useI18n();
    const { resolvedTheme } = useTheme();
    const editorRef = useRef<MonacoEditor | null>(null);
    const monacoRef = useRef<MonacoApi | null>(null);
    const modelRef = useRef<MonacoModel | null>(null);
    const validationSeqRef = useRef(0);
    // Bound once in onMount; deref the latest handler so we never re-bind.
    const onSaveRef = useRef(onSave);
    onSaveRef.current = onSave;

    useImperativeHandle(
      ref,
      () => ({
        jumpToLine(line: number) {
          const editor = editorRef.current;
          if (!editor) return;
          editor.revealLineInCenter(line);
          editor.setPosition({ lineNumber: line, column: 1 });
          editor.focus();
        },
      }),
      [],
    );

    const [backendDiagnostics, setBackendDiagnostics] = useState<
      OxiDnsNextYamlDiagnostic[]
    >([]);
    const context = useMemo(
      () => ({
        variant,
        locale,
        plugins,
        pluginKind,
        fields,
        currentPluginName,
      }),
      [variant, locale, plugins, pluginKind, fields, currentPluginName],
    );
    const theme =
      resolvedTheme === "light" ? "oxidns-next-yaml-light" : "oxidns-next-yaml-dark";

    // beforeMount runs synchronously before monaco.editor.create(), ensuring
    // custom themes are defined before the editor tries to apply them via the
    // theme prop. Without this, the editor briefly renders with vs-dark / vs.
    const handleBeforeMount = (monaco: MonacoApi) => {
      registerOxiDnsNextYamlLanguage(monaco);
    };

    const handleMount: OnMount = (editor, monaco) => {
      editorRef.current = editor;
      monacoRef.current = monaco;
      const model = editor.getModel();
      modelRef.current = model;

      registerOxiDnsNextYamlLanguage(monaco);
      // KeyMod.CtrlCmd maps to ⌘ on macOS and Ctrl on Windows/Linux
      // automatically, so the save shortcut is OS-correct by construction and
      // the browser "save page" dialog is suppressed while the editor is focused.
      editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
        onSaveRef.current?.();
      });
      if (model) {
        setOxiDnsNextYamlModelContext(model, context);
        updateOxiDnsNextYamlMarkers(monaco, model, context, backendDiagnostics);
      }
    };

    useEffect(() => {
      const model = modelRef.current;
      if (!model) return;
      setOxiDnsNextYamlModelContext(model, context);
    }, [context]);

    useEffect(() => {
      const monaco = monacoRef.current;
      const model = modelRef.current;
      if (!monaco || !model) return;
      updateOxiDnsNextYamlMarkers(
        monaco,
        model,
        context,
        variant === "config" && !readOnly && backendValidation
          ? backendDiagnostics
          : [],
      );
    }, [
      backendValidation,
      backendDiagnostics,
      context,
      readOnly,
      value,
      variant,
    ]);

    useEffect(() => {
      if (variant !== "config" || readOnly || !backendValidation) {
        return;
      }

      const seq = validationSeqRef.current + 1;
      validationSeqRef.current = seq;
      const timer = window.setTimeout(() => {
        void validateConfigText(value)
          .then(() => {
            if (validationSeqRef.current === seq) setBackendDiagnostics([]);
          })
          .catch((error: unknown) => {
            if (validationSeqRef.current !== seq) return;
            if (error instanceof ConfigValidationError) {
              setBackendDiagnostics(
                error.diagnosticDetails.length > 0
                  ? error.diagnosticDetails
                  : error.diagnostics.map((message) => ({ message })),
              );
              return;
            }
            setBackendDiagnostics([
              {
                message:
                  error instanceof Error
                    ? error.message
                    : t(WEBUI.configEditor.configValidationFailed),
              },
            ]);
          });
      }, 800);

      return () => window.clearTimeout(timer);
    }, [backendValidation, readOnly, t, value, variant]);

    useEffect(() => {
      const model = modelRef.current;
      return () => {
        if (model) clearOxiDnsNextYamlModelContext(model);
      };
    }, []);

    useEffect(() => {
      const monaco = monacoRef.current;
      if (!monaco) return;
      monaco.editor.setTheme(theme);
    }, [theme]);

    return (
      <div
        className={cn(
          "relative overflow-hidden rounded-md border bg-muted/30 font-mono text-sm [&_.monaco-editor_.sticky-widget]:bg-background [&_.monaco-editor_.sticky-widget_.sticky-widget-line-numbers]:bg-background [&>section]:min-h-[inherit]",
          className,
        )}
      >
        <Editor
          height="100%"
          value={value}
          defaultLanguage="yaml"
          language="yaml"
          theme={theme}
          beforeMount={handleBeforeMount}
          onMount={handleMount}
          onChange={(nextValue) => onChange?.(nextValue ?? "")}
          options={{
            readOnly,
            tabSize: 2,
            insertSpaces: true,
            detectIndentation: false,
            minimap: { enabled: false },
            lineNumbers: lineNumbers ? "on" : "off",
            lineNumbersMinChars: 4,
            lineDecorationsWidth: 10,
            glyphMargin: false,
            folding: true,
            scrollBeyondLastLine: false,
            wordWrap: "on",
            wrappingIndent: "same",
            automaticLayout: true,
            fontSize: 14,
            lineHeight: 24,
            fontFamily:
              "JetBrains Mono, ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace",
            letterSpacing: 0,
            padding: { top: 12, bottom: 12 },
            renderLineHighlight: "line",
            stickyScroll: { enabled: true },
            scrollbar: {
              verticalScrollbarSize: 10,
              horizontalScrollbarSize: 10,
            },
            quickSuggestions: {
              other: true,
              comments: false,
              strings: true,
            },
            suggestOnTriggerCharacters: true,
            fixedOverflowWidgets: true,
            contextmenu: true,
            readOnlyMessage: { value: t(WEBUI.common.readOnly) },
          }}
        />
      </div>
    );
  },
);
