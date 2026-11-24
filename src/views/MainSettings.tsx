import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ALT_LABEL, formatShortcutLabel, MOD_LABEL, OS_LABEL } from "../utils/platform";
import { listenToWindow } from "../utils/windowEvents";
import {
  ArrowRight,
  Camera,
  Check,
  ChevronDown,
  ChevronRight,
  Clipboard,
  Copy,
  Cpu,
  Download,
  ExternalLink,
  Eye,
  EyeOff,
  History,
  Keyboard,
  Languages,
  Monitor,
  Moon,
  RotateCcw,
  ScanLine,
  Search,
  Sparkles,
  Sun,
  Trash2,
  Upload,
  Volume2,
  X,
  Zap,
} from "lucide-react";
import {
  AppConfig,
  AskQuickAction,
  DEFAULT_ASK_QUICK_ACTIONS,
  ENGINE_OPTIONS,
  PROVIDER_PRESETS,
  SHORTCUT_PRESETS,
  ShortcutStatus,
  StreamPreviewEvent,
  SystemProxyInfo,
  TARGET_LANGUAGES,
  ThemeMode,
  TranslateEngine,
  TranslationResult,
} from "../types";

import { AskActionsSettings } from "../components/AskActionsSettings";
import {
  HistoryScrollMode,
  HistoryScrollStage,
  HistoryViewMode,
} from "../components/HistoryScrollStage";
import { LocalOcrSettings } from "../components/LocalOcrSettings";
import { Popconfirm } from "../components/Popconfirm";
import { ShortcutRow } from "../components/ShortcutRow";
import { SlidingSegmented } from "../components/SlidingSegmented";
import { UpdateButton } from "./UpdateButton";
import { UpdateDialog } from "./UpdateDialog";
import { useAppUpdate } from "../hooks/useAppUpdate";

type WorkspacePane = "shortcuts" | "engines" | "system" | "text" | "history";

const HISTORY_VIEW_STORAGE_KEY = "luma.historyViewMode";

function loadSavedHistoryViewMode(): HistoryViewMode {
  try {
    const raw = window.localStorage.getItem(HISTORY_VIEW_STORAGE_KEY);
    if (raw === "list" || raw === "stacked" || raw === "coverflow" || raw === "deck") {
      return raw;
    }
  } catch {
    // ignore storage errors
  }
  return "stacked";
}

const DEMO_HISTORY_ITEMS: TranslationResult[] = [
  {
    id: -101,
    mode: "translate",
    engine: "ai",
    sourceLang: "EN",
    targetLang: "简体中文",
    originalText: "Hold Tab while selecting to replace only text blocks in place without darkening the screen.",
    translatedText: "零遮罩透视截图与原位回填：选区外部 100% 透明保持工作上下文，按住 Tab 仅替换识别到的文字块。",
    elapsedMs: 310,
    model: "gpt-4o-mini",
    imageDataUrl: "",
    createdAtMs: Date.now() - 4 * 60_000,
  },
  {
    id: -102,
    mode: "translate",
    engine: "ai",
    sourceLang: "EN",
    targetLang: "简体中文",
    originalText: "Multiple captures fly smoothly into the corner stack and fan out physically on hover.",
    translatedText: "角落叠堆与悬停物理扇开：连续截取多张参考图后自动汇入桌面右下角，悬停即扇形展开，点开抽屉支持拖拽波纹让位。",
    elapsedMs: 285,
    model: "gpt-4o-mini",
    imageDataUrl: "",
    createdAtMs: Date.now() - 18 * 60_000,
  },
  {
    id: -103,
    mode: "ocr",
    engine: "ai",
    sourceLang: "AUTO",
    targetLang: "简体中文",
    originalText: "",
    translatedText: "本地离线高精度 OCR 识别：无需联网即可秒级提取公式、代码报错与多语言表格，支持一键复制或转入 AI 追问。",
    elapsedMs: 142,
    model: "RapidOCR v4",
    imageDataUrl: "",
    createdAtMs: Date.now() - 42 * 60_000,
  },
  {
    id: -104,
    mode: "translate",
    engine: "deepl",
    sourceLang: "EN",
    targetLang: "简体中文",
    originalText: "requireSignedVersion: true · NSIS allowDowngrades: false",
    translatedText: "签名校验与后台自动提醒：后台静默检查已签名的 latest.json，发现新版仅通过托盘与轻提示告知，由你确认后安装。",
    elapsedMs: 260,
    model: "DeepL",
    imageDataUrl: "",
    createdAtMs: Date.now() - 68 * 60_000,
  },
  {
    id: -105,
    mode: "translate",
    engine: "ai",
    sourceLang: "EN",
    targetLang: "简体中文",
    originalText: "transparent: true · decorations: false · skipTaskbar: true",
    translatedText: "轻浮于应用层之上，绝不霸屏：非黑 3D 立体按键与贴边浮窗设计，让翻译与识字隐于工作流之中、随时呼之即来。",
    elapsedMs: 295,
    model: "gpt-4o-mini",
    imageDataUrl: "",
    createdAtMs: Date.now() - 125 * 60_000,
  },
];

const DEFAULT_CONFIG: AppConfig = {
  shortcut: "Alt+Q",
  clipboard_shortcut: "",
  selection_shortcut: "",
  target_lang: "简体中文",
  translate_engine: "ai",
  google_api_key: "",
  deepl_api_key: "",
  api_base_url: "https://api.openai.com/v1",
  api_key: "",
  model: "gpt-4o-mini",
  custom_prompt: "",
  glossary: "",
  auto_translate_on_release: false,
  start_minimized: false,
  launch_at_startup: false,
  max_stack_size: 6,
  clear_stack_after_output: false,
  theme: "system",
  ask_quick_actions: DEFAULT_ASK_QUICK_ACTIONS,
  ocr_engine: "auto",
  ocr_lang: "ch",
  ocr_download_url: "",
  proxy_url: "",
  update_feed_url: "",
};

const CAPTURE_PRESETS = SHORTCUT_PRESETS.map((sc) => ({
  value: sc,
  label: formatShortcutLabel(sc),
}));

const CLIPBOARD_PRESETS = [
  { value: "", label: "关闭" },
  { value: "Alt+W", label: formatShortcutLabel("Alt+W") },
  { value: "Ctrl+Alt+C", label: formatShortcutLabel("Ctrl+Alt+C") },
  { value: "Ctrl+Shift+Q", label: formatShortcutLabel("Ctrl+Shift+Q") },
];

const SELECTION_PRESETS = [
  { value: "", label: "关闭" },
  { value: "Space", label: "Space" },
  { value: "Tab", label: "Tab" },
  { value: "Alt+E", label: `${ALT_LABEL}+E` },
];

function formatKeyEventToAccelerator(
  e: KeyboardEvent,
  allowSingleKeys = false
): string | null {
  const modKeys = ["Control", "Shift", "Alt", "Meta"];
  if (modKeys.includes(e.key)) return null;

  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");

  let keyName = "";
  const code = e.code;
  if (/^Key[A-Z]$/.test(code)) {
    keyName = code.slice(3);
  } else if (/^Digit[0-9]$/.test(code)) {
    keyName = code.slice(5);
  } else if (/^F([1-9]|1[0-2])$/.test(code)) {
    keyName = code;
  } else if (code === "Space") {
    keyName = "Space";
  } else if (code === "Tab") {
    keyName = "Tab";
  } else if (code === "Backquote") {
    keyName = "`";
  } else if (code === "Minus") {
    keyName = "-";
  } else if (code === "Equal") {
    keyName = "=";
  } else if (code === "BracketLeft") {
    keyName = "[";
  } else if (code === "BracketRight") {
    keyName = "]";
  } else if (code === "Comma") {
    keyName = ",";
  } else if (code === "Period") {
    keyName = ".";
  } else if (code === "Slash") {
    keyName = "/";
  } else if (code.startsWith("Arrow")) {
    keyName = code.slice(5);
  }

  if (!keyName) return null;
  const isFnKey = /^F([1-9]|1[0-2])$/.test(keyName);
  const isSingleTrigger = allowSingleKeys && (keyName === "Space" || keyName === "Tab");
  if (mods.length === 0 && !isFnKey && !isSingleTrigger) return null;
  return [...mods, keyName].join("+");
}

function splitQuickKeycaps(shortcut: string): string[] {
  const formatted = formatShortcutLabel(shortcut.trim());
  return formatted ? formatted.split("+").filter(Boolean) : [];
}

export function MainSettings() {
  const [pane, setPane] = useState<WorkspacePane>("shortcuts");
  const [config, setConfig] = useState<AppConfig>(DEFAULT_CONFIG);
  const [savedConfig, setSavedConfig] = useState<AppConfig | null>(null);
  const dirty = savedConfig !== null && JSON.stringify(config) !== JSON.stringify(savedConfig);
  const startupChanged = savedConfig !== null && config.start_minimized !== savedConfig.start_minimized;

  const [history, setHistory] = useState<TranslationResult[]>([]);
  const [exitingHistoryIds, setExitingHistoryIds] = useState<Record<number, true>>({});
  const [showAiKey, setShowAiKey] = useState(false);
  const [showDeepLKey, setShowDeepLKey] = useState(false);
  const [showPromptGlossary, setShowPromptGlossary] = useState(false);
  const [saving, setSaving] = useState(false);
  const appUpdate = useAppUpdate({ dirty: dirty || saving });
  const [testingTarget, setTestingTarget] = useState<string | null>(null);
  const [status, setStatus] = useState<{ kind: "ok" | "err" | "idle"; text: string }>({
    kind: "idle",
    text: "",
  });
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const [shortcutStatus, setShortcutStatus] = useState<ShortcutStatus | null>(null);
  const [appVersion, setAppVersion] = useState<string>("");
  const [historyQuery, setHistoryQuery] = useState("");
  const [historyModeFilter, setHistoryModeFilter] = useState<"all" | "translate" | "ocr">("all");
  const [historyViewMode, setHistoryViewMode] = useState<HistoryViewMode>(loadSavedHistoryViewMode);
  const [historyDemoMode, setHistoryDemoMode] = useState(false);

  const changeHistoryViewMode = (next: HistoryViewMode) => {
    setHistoryViewMode(next);
    try {
      window.localStorage.setItem(HISTORY_VIEW_STORAGE_KEY, next);
    } catch {
      // ignore storage errors
    }
  };

  useEffect(() => {
    if (history.length > 0 && historyDemoMode) {
      setHistoryDemoMode(false);
    }
  }, [history.length, historyDemoMode]);
  const [recordingTarget, setRecordingTarget] = useState<"capture" | "clipboard" | "selection" | null>(null);
  const [systemProxy, setSystemProxy] = useState<SystemProxyInfo | null>(null);
  const [proxyMode, setProxyMode] = useState<"system" | "custom">("system");
  const [showUpdateMirror, setShowUpdateMirror] = useState(false);

  const savedAskActionsJsonRef = useRef<string | null>(null);
  useEffect(() => {
    savedAskActionsJsonRef.current = savedConfig
      ? JSON.stringify(savedConfig.ask_quick_actions)
      : null;
  }, [savedConfig]);

  const [textInput, setTextInput] = useState("");
  const [textEngine, setTextEngine] = useState<TranslateEngine>("ai");
  const [textTargetLang, setTextTargetLang] = useState<string>("简体中文");
  const [textResult, setTextResult] = useState<TranslationResult | null>(null);
  const [textStreamPreview, setTextStreamPreview] = useState("");
  const [textBusy, setTextBusy] = useState(false);
  const [textError, setTextError] = useState<string | null>(null);
  const [copiedTextOut, setCopiedTextOut] = useState(false);
  const textReqIdRef = useRef(0);

  const [historyLoadError, setHistoryLoadError] = useState<string | null>(null);
  const [configLoadError, setConfigLoadError] = useState<string | null>(null);

  const toastTimerRef = useRef<number | null>(null);
  useEffect(() => {
    if (toastTimerRef.current !== null) {
      window.clearTimeout(toastTimerRef.current);
      toastTimerRef.current = null;
    }
    if (status.text && status.kind === "ok") {
      toastTimerRef.current = window.setTimeout(() => {
        setStatus({ kind: "idle", text: "" });
      }, 3200);
    }
    return () => {
      if (toastTimerRef.current !== null) {
        window.clearTimeout(toastTimerRef.current);
      }
    };
  }, [status]);

  const loadHistory = async () => {
    try {
      const list = await invoke<TranslationResult[]>("get_history");
      setHistory(list);
      setHistoryLoadError(null);
    } catch (err) {
      console.error(err);
      setHistoryLoadError(String(err));
    }
  };

  useEffect(() => {
    let cancelled = false;
    const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
    const retryInvoke = async <T,>(cmd: string, attempts = 5): Promise<T> => {
      let lastErr: unknown;
      for (let i = 0; i < attempts; i++) {
        try {
          return await invoke<T>(cmd);
        } catch (err) {
          lastErr = err;
          if (cancelled || i === attempts - 1) break;
          await sleep(140 * (i + 1));
        }
      }
      throw lastErr;
    };

    retryInvoke<string>("get_app_version")
      .then((ver) => {
        if (!cancelled) setAppVersion(ver);
      })
      .catch(console.error);

    retryInvoke<AppConfig>("get_config")
      .then((cfg) => {
        if (cancelled) return;
        setConfig(cfg);
        setSavedConfig(cfg);
        setProxyMode(cfg.proxy_url.trim() ? "custom" : "system");
        setShowUpdateMirror(Boolean(cfg.update_feed_url.trim()));
        setShowPromptGlossary(Boolean(cfg.custom_prompt.trim() || cfg.glossary.trim()));
        setConfigLoadError(null);
        setTextEngine(cfg.translate_engine);
        setTextTargetLang(cfg.target_lang);
        document.documentElement.dataset.theme =
          cfg.theme === "light" || cfg.theme === "dark" ? cfg.theme : "system";
      })
      .catch((err) => {
        console.error(err);
        if (!cancelled) {
          setConfigLoadError(`读取设置失败: ${String(err)}`);
          setStatus({ kind: "err", text: `读取设置失败: ${String(err)}` });
        }
      });

    retryInvoke<ShortcutStatus>("get_shortcut_status")
      .then((st) => {
        if (!cancelled) setShortcutStatus(st);
      })
      .catch(console.error);

    retryInvoke<SystemProxyInfo>("get_system_proxy_info")
      .then((info) => {
        if (!cancelled) setSystemProxy(info);
      })
      .catch(console.error);

    loadHistory();

    let unlistenHistory: (() => void) | undefined;
    let unlistenShortcut: (() => void) | undefined;
    let unlistenCaptureErr: (() => void) | undefined;
    let unlistenTextStream: (() => void) | undefined;
    let unlistenAskActions: (() => void) | undefined;

    listen("history:updated", () => {
      loadHistory();
    }).then((fn) => {
      unlistenHistory = fn;
    });

    listen<ShortcutStatus>("shortcut:status", (event) => {
      setShortcutStatus(event.payload);
    }).then((fn) => {
      unlistenShortcut = fn;
    });

    listen<string>("capture:error", (event) => {
      setStatus({ kind: "err", text: event.payload });
    }).then((fn) => {
      unlistenCaptureErr = fn;
    });

    listenToWindow<StreamPreviewEvent>("main:text-stream", (event) => {
      if (event.payload.jobId === textReqIdRef.current) {
        setTextStreamPreview(event.payload.text);
      }
    }).then((fn) => {
      unlistenTextStream = fn;
    });

    listen<AskQuickAction[]>("ask-actions:updated", (event) => {
      const nextActions = event.payload;
      setSavedConfig((prevSaved) =>
        prevSaved ? { ...prevSaved, ask_quick_actions: nextActions } : prevSaved
      );
      setConfig((prev) => {
        const savedJson = savedAskActionsJsonRef.current;
        if (savedJson === null || JSON.stringify(prev.ask_quick_actions) === savedJson) {
          return { ...prev, ask_quick_actions: nextActions };
        }
        return prev;
      });
    }).then((fn) => {
      unlistenAskActions = fn;
    });

    return () => {
      cancelled = true;
      if (unlistenHistory) unlistenHistory();
      if (unlistenShortcut) unlistenShortcut();
      if (unlistenCaptureErr) unlistenCaptureErr();
      if (unlistenTextStream) unlistenTextStream();
      if (unlistenAskActions) unlistenAskActions();
    };
  }, []);

  useEffect(() => {
    if (!recordingTarget) return;
    const active = document.activeElement as HTMLElement | null;
    active?.blur?.();
    const allowSingle = recordingTarget === "selection";
    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setRecordingTarget(null);
        return;
      }
      const formatted = formatKeyEventToAccelerator(e, allowSingle);
      if (formatted) {
        if (recordingTarget === "capture") {
          setConfig((prev) => ({ ...prev, shortcut: formatted }));
        } else if (recordingTarget === "selection") {
          setConfig((prev) => ({ ...prev, selection_shortcut: formatted }));
        } else {
          setConfig((prev) => ({ ...prev, clipboard_shortcut: formatted }));
        }
        setRecordingTarget(null);
        setStatus({ kind: "ok", text: `已录制 ${formatShortcutLabel(formatted)}，保存后生效` });
      } else if (!["Control", "Shift", "Alt", "Meta"].includes(e.key)) {
        setStatus({
          kind: "err",
          text: allowSingle
            ? `划词键需为 Space / Tab、F1-F12 或 ${MOD_LABEL} / ${ALT_LABEL} / Shift 组合键`
            : `快捷键需搭配 ${MOD_LABEL} / ${ALT_LABEL} / Shift 或 F1-F12`,
        });
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [recordingTarget]);

  const applyThemeSelection = (nextTheme: ThemeMode) => {
    setConfig((prev) => ({ ...prev, theme: nextTheme }));
    document.documentElement.dataset.theme = nextTheme;
    invoke("preview_theme", { theme: nextTheme }).catch(console.error);
  };

  const handleSave = async () => {
    if (saving || !dirty) return;
    setSaving(true);
    setStatus({ kind: "idle", text: "" });
    try {
      const saved = await invoke<AppConfig>("update_config", { newConfig: config });
      setConfig(saved);
      setSavedConfig(saved);
      setProxyMode(saved.proxy_url.trim() ? "custom" : "system");
      if (saved.translate_engine !== savedConfig?.translate_engine) setTextEngine(saved.translate_engine);
      if (saved.target_lang !== savedConfig?.target_lang) setTextTargetLang(saved.target_lang);
      setStatus({ kind: "ok", text: startupChanged ? "已保存，静默启动将在下次生效" : "已保存" });
    } catch (err) {
      setStatus({ kind: "err", text: String(err) });
    } finally {
      setSaving(false);
    }
  };

  const handleDiscard = () => {
    if (!savedConfig) return;
    setConfig(savedConfig);
    setProxyMode(savedConfig.proxy_url.trim() ? "custom" : "system");
    setShowUpdateMirror(Boolean(savedConfig.update_feed_url.trim()));
    document.documentElement.dataset.theme =
      savedConfig.theme === "light" || savedConfig.theme === "dark" ? savedConfig.theme : "system";
    invoke("preview_theme", { theme: savedConfig.theme }).catch(console.error);
    setStatus({ kind: "ok", text: "已放弃未保存更改" });
  };

  const handleSaveRef = useRef(handleSave);
  handleSaveRef.current = handleSave;

  useEffect(() => {
    if (recordingTarget) return;
    const onSaveKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void handleSaveRef.current();
      }
    };
    window.addEventListener("keydown", onSaveKey);
    return () => window.removeEventListener("keydown", onSaveKey);
  }, [recordingTarget]);

  const handleTest = async (target: "ai" | "google" | "deepl" | "update") => {
    setTestingTarget(target);
    setStatus({
      kind: "idle",
      text: target === "update" ? "正在测试代理与更新源…" : "正在测试连接…",
    });
    if (target === "update") {
      invoke<SystemProxyInfo>("get_system_proxy_info")
        .then(setSystemProxy)
        .catch(() => {});
    }
    try {
      const msg = await invoke<string>("test_api_connection", { config, target });
      setStatus({ kind: "ok", text: msg });
    } catch (err) {
      setStatus({ kind: "err", text: String(err) });
    } finally {
      setTestingTarget(null);
    }
  };

  const handleCaptureNow = async () => {
    try {
      await invoke("start_capture", { hideMain: true });
    } catch (err) {
      setStatus({ kind: "err", text: String(err) });
    }
  };

  const handleClipboardImage = async () => {
    try {
      await invoke("analyze_clipboard_image", { mode: "translate" });
    } catch (err) {
      setStatus({ kind: "err", text: String(err) });
    }
  };

  const handleTranslateText = async () => {
    if (!textInput.trim() || textBusy) return;
    const reqId = ++textReqIdRef.current;
    setTextBusy(true);
    setTextError(null);
    setTextResult(null);
    setTextStreamPreview("");
    try {
      const res = await invoke<TranslationResult>("translate_text_direct", {
        text: textInput,
        engineOverride: textEngine,
        targetLangOverride: textTargetLang,
        requestId: reqId,
      });
      if (textReqIdRef.current === reqId) {
        setTextResult(res);
        setTextStreamPreview("");
      }
    } catch (err) {
      if (textReqIdRef.current === reqId) {
        setTextError(String(err));
        setTextStreamPreview("");
      }
    } finally {
      if (textReqIdRef.current === reqId) {
        setTextBusy(false);
      }
    }
  };

  const handleExportConfig = async () => {
    try {
      const ok = await invoke<boolean>("export_config_json");
      if (ok) {
        setStatus({ kind: "ok", text: "已导出配置备份 (JSON)" });
      }
    } catch (err) {
      setStatus({ kind: "err", text: String(err) });
    }
  };

  const handleImportConfig = async () => {
    try {
      const imported = await invoke<AppConfig | null>("import_config_json");
      if (imported) {
        setConfig(imported);
        setSavedConfig(imported);
        setProxyMode(imported.proxy_url.trim() ? "custom" : "system");
        setShowUpdateMirror(Boolean(imported.update_feed_url.trim()));
        setShowPromptGlossary(Boolean(imported.custom_prompt.trim() || imported.glossary.trim()));
        setTextEngine(imported.translate_engine);
        setTextTargetLang(imported.target_lang);
        setStatus({ kind: "ok", text: "已导入配置并立即生效" });
      }
    } catch (err) {
      setStatus({ kind: "err", text: String(err) });
    }
  };

  const speakDirectText = (txt: string) => {
    if (typeof window === "undefined" || !("speechSynthesis" in window) || !txt.trim()) return;
    window.speechSynthesis.cancel();
    window.speechSynthesis.speak(new SpeechSynthesisUtterance(txt.trim()));
  };

  const changeAiEndpoint = (baseUrl: string, model?: string) => {
    setConfig((prev) => {
      let sameOrigin = false;
      try {
        sameOrigin = new URL(prev.api_base_url).origin === new URL(baseUrl).origin;
      } catch {
        // Incomplete URL has no origin to retain a key for.
      }
      return {
        ...prev,
        api_base_url: baseUrl,
        api_key: sameOrigin ? prev.api_key : "",
        model: model ?? prev.model,
      };
    });
  };

  const copyHistory = async (item: TranslationResult) => {
    await navigator.clipboard.writeText(item.translatedText);
    setCopiedId(item.id);
    setTimeout(() => setCopiedId((cur) => (cur === item.id ? null : cur)), 1400);
  };

  const deleteHistoryItem = async (id: number) => {
    if (exitingHistoryIds[id]) return;
    setExitingHistoryIds((prev) => ({ ...prev, [id]: true }));
    const reduced =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    if (!reduced) {
      await new Promise((r) => window.setTimeout(r, 175));
    }
    try {
      const next = await invoke<TranslationResult[]>("delete_history_item", { id });
      setHistory(next);
    } catch (err) {
      setStatus({ kind: "err", text: String(err) });
    } finally {
      setExitingHistoryIds((prev) => {
        if (!(id in prev)) return prev;
        const next = { ...prev };
        delete next[id];
        return next;
      });
    }
  };

  const exportHistory = async (format: "markdown" | "json") => {
    try {
      const ok = await invoke<boolean>("export_history", { format });
      if (ok) {
        setStatus({
          kind: "ok",
          text: `已导出历史 (${format === "json" ? "JSON" : "Markdown"})`,
        });
      }
    } catch (err) {
      setStatus({ kind: "err", text: String(err) });
    }
  };

  const filteredHistory = history.filter((item) => {
    if (historyModeFilter !== "all" && item.mode !== historyModeFilter) {
      return false;
    }
    const q = historyQuery.trim().toLowerCase();
    if (!q) return true;
    return (
      item.translatedText.toLowerCase().includes(q) ||
      item.originalText.toLowerCase().includes(q) ||
      item.model.toLowerCase().includes(q) ||
      item.sourceLang.toLowerCase().includes(q) ||
      item.targetLang.toLowerCase().includes(q)
    );
  });

  const formatTimestamp = (ms: number) => {
    if (!ms) return "";
    try {
      return new Date(ms).toLocaleString("zh-CN", {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
      });
    } catch {
      return "";
    }
  };

  const selectionNormalized = config.selection_shortcut.trim().toLowerCase();
  const isSmartSingleSelection =
    selectionNormalized === "space" || selectionNormalized === "tab";

  const activeCaptureCaps = splitQuickKeycaps(
    shortcutStatus?.active ? shortcutStatus.shortcut : config.shortcut
  );
  const activeClipCaps = splitQuickKeycaps(
    shortcutStatus?.clipboardShortcut || config.clipboard_shortcut
  );

  const cycleQuickTheme = () => {
    const order: ThemeMode[] = ["light", "dark", "system"];
    const next = order[(order.indexOf(config.theme) + 1) % order.length];
    applyThemeSelection(next);
  };

  const themeIcon =
    config.theme === "dark" ? (
      <Moon size={13} />
    ) : config.theme === "light" ? (
      <Sun size={13} />
    ) : (
      <Monitor size={13} />
    );

  const themeLabel =
    config.theme === "dark" ? "夜间" : config.theme === "light" ? "浅色" : "跟随系统";

  const glossaryCount = config.glossary
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l && (l.includes("=") || l.includes("=>"))).length;

  return (
    <div className="v3-app-shell">
      {/* ==================== Left Navigation Sidebar ==================== */}
      <aside className="v3-sidebar" aria-label="主导航">
        <div className="sidebar-brand">
          <div className="brand-badge">
            <ScanLine size={16} aria-hidden="true" />
          </div>
          <div className="sidebar-brand-text">
            <div className="sidebar-brand-row">
              <span className="sidebar-brand-name">Luma</span>
              {appVersion && <span className="sidebar-ver">v{appVersion}</span>}
            </div>
            <div className="sidebar-hotkey-status">
              <span
                className={`ready-dot${shortcutStatus?.active ? " on" : ""}`}
                aria-hidden="true"
              />
              <span>
                {shortcutStatus?.active ? "热键就绪" : "等待就绪"}
              </span>
            </div>
          </div>
        </div>

        <div className="sidebar-cta-stack">
          <button
            type="button"
            className="btn btn-primary sidebar-cta-btn"
            onClick={handleCaptureNow}
          >
            <span className="sidebar-cta-left">
              <Camera size={14} /> 截图
            </span>
            {activeCaptureCaps.length > 0 && (
              <span className="sidebar-cta-kbds">
                {activeCaptureCaps.map((k, i) => (
                  <kbd key={`${k}-${i}`} className="kbd-cap kbd-cap-inv">
                    {k}
                  </kbd>
                ))}
              </span>
            )}
          </button>

          <button
            type="button"
            className="btn sidebar-cta-btn sidebar-cta-sub"
            onClick={handleClipboardImage}
            title="识别并翻译剪贴板中的图片"
          >
            <span className="sidebar-cta-left">
              <Clipboard size={13} /> 译剪贴板
            </span>
            {activeClipCaps.length > 0 && (
              <span className="sidebar-cta-kbds">
                {activeClipCaps.map((k, i) => (
                  <kbd key={`${k}-${i}`} className="kbd-cap">
                    {k}
                  </kbd>
                ))}
              </span>
            )}
          </button>
        </div>

        <nav className="sidebar-nav">
          <div className="sidebar-group">
            <div className="sidebar-group-title">工作台</div>
            <button
              type="button"
              className={`sidebar-nav-item${pane === "text" ? " active" : ""}`}
              aria-current={pane === "text" ? "page" : undefined}
              onClick={() => setPane("text")}
            >
              <span className="sidebar-nav-left">
                <Languages size={14} />
                <span>翻译</span>
              </span>
            </button>
            <button
              type="button"
              className={`sidebar-nav-item${pane === "history" ? " active" : ""}`}
              aria-current={pane === "history" ? "page" : undefined}
              onClick={() => {
                setPane("history");
                loadHistory();
              }}
            >
              <span className="sidebar-nav-left">
                <History size={14} />
                <span>历史</span>
              </span>
              <span className="sidebar-nav-badge">{history.length}</span>
            </button>
          </div>

          <div className="sidebar-group">
            <div className="sidebar-group-title">设置</div>
            <button
              type="button"
              className={`sidebar-nav-item${pane === "shortcuts" ? " active" : ""}`}
              aria-current={pane === "shortcuts" ? "page" : undefined}
              onClick={() => setPane("shortcuts")}
            >
              <span className="sidebar-nav-left">
                <Keyboard size={14} />
                <span>快捷键</span>
              </span>
            </button>
            <button
              type="button"
              className={`sidebar-nav-item${pane === "engines" ? " active" : ""}`}
              aria-current={pane === "engines" ? "page" : undefined}
              onClick={() => setPane("engines")}
            >
              <span className="sidebar-nav-left">
                <Sparkles size={14} />
                <span>翻译引擎</span>
              </span>
            </button>
            <button
              type="button"
              className={`sidebar-nav-item${pane === "system" ? " active" : ""}`}
              aria-current={pane === "system" ? "page" : undefined}
              onClick={() => setPane("system")}
            >
              <span className="sidebar-nav-left">
                <Cpu size={14} />
                <span>识别与外观</span>
              </span>
            </button>
          </div>
        </nav>

        <div className="sidebar-footer">
          <UpdateButton update={appUpdate} variant="sidebar" />
          <button
            type="button"
            className="sidebar-theme-btn"
            onClick={cycleQuickTheme}
            title="切换浅色 / 夜间 / 跟随系统（即时预览）"
          >
            <span className="sidebar-update-left">
              {themeIcon}
              <span>外观：{themeLabel}</span>
            </span>
          </button>
        </div>
      </aside>

      {/* ==================== Right Workspace Canvas ==================== */}
      <main className="v3-workspace" aria-busy={saving}>
        <div
          className={`v3-workspace-scroll${
            pane === "history" &&
            historyViewMode !== "list" &&
            (filteredHistory.length > 0 || (history.length === 0 && historyDemoMode))
              ? " is-history-stage"
              : ""
          }`}
        >
          {/* PANE 1: SHORTCUTS & INTERACTION */}
          {pane === "shortcuts" && (
            <div key="shortcuts" className="v3-pane">
              <div className="v3-pane-head">
                <div>
                  <h1 className="v3-pane-title">快捷键</h1>
                  <p className="v3-pane-sub">点击键帽即可录制，按 Esc 取消</p>
                </div>
                <div className="v3-head-control">
                  <label className="v3-inline-label" htmlFor="default-target-lang">
                    目标语言
                  </label>
                  <select
                    id="default-target-lang"
                    className="select v3-compact-select"
                    value={config.target_lang}
                    onChange={(e) =>
                      setConfig({ ...config, target_lang: e.target.value })
                    }
                  >
                    {TARGET_LANGUAGES.map((lang) => (
                      <option key={lang} value={lang}>
                        {lang}
                      </option>
                    ))}
                  </select>
                </div>
              </div>

              {configLoadError && <div className="error-box">{configLoadError}</div>}

              <section className="card v3-card">
                <div className="card-title">
                  <span>全局快捷键</span>
                </div>

                <ShortcutRow
                  label="截图快捷键"
                  desc="唤起无遮罩框选截图，按住 Tab 原位译文"
                  value={config.shortcut}
                  savedValue={shortcutStatus?.shortcut}
                  recording={recordingTarget === "capture"}
                  presets={CAPTURE_PRESETS}
                  error={
                    shortcutStatus && !shortcutStatus.active && shortcutStatus.error
                      ? shortcutStatus.error
                      : null
                  }
                  onChange={(next) => setConfig((prev) => ({ ...prev, shortcut: next }))}
                  onToggleRecord={() =>
                    setRecordingTarget((v) => (v === "capture" ? null : "capture"))
                  }
                />

                <ShortcutRow
                  label="剪贴板速译"
                  desc="复制文字后一键浮窗翻译"
                  value={config.clipboard_shortcut}
                  savedValue={shortcutStatus?.clipboardShortcut ?? ""}
                  recording={recordingTarget === "clipboard"}
                  optional
                  presets={CLIPBOARD_PRESETS}
                  error={shortcutStatus?.clipboardError ?? null}
                  onChange={(next) =>
                    setConfig((prev) => ({ ...prev, clipboard_shortcut: next }))
                  }
                  onToggleRecord={() =>
                    setRecordingTarget((v) => (v === "clipboard" ? null : "clipboard"))
                  }
                />

                <ShortcutRow
                  label="划词翻译"
                  desc="选中任意软件文字后轻按即译"
                  value={config.selection_shortcut}
                  savedValue={shortcutStatus?.selectionShortcut ?? ""}
                  recording={recordingTarget === "selection"}
                  recordingPrompt="按下快捷键（支持 Space / Tab）"
                  optional
                  presets={SELECTION_PRESETS}
                  hint={
                    isSmartSingleSelection
                      ? `选中文字后轻按 ${selectionNormalized === "space" ? "Space" : "Tab"} 即可翻译，正常输入与全屏游戏自动避让。`
                      : null
                  }
                  error={shortcutStatus?.selectionError ?? null}
                  onChange={(next) =>
                    setConfig((prev) => ({ ...prev, selection_shortcut: next }))
                  }
                  onToggleRecord={() =>
                    setRecordingTarget((v) => (v === "selection" ? null : "selection"))
                  }
                />
              </section>

              <section className="card v3-card">
                <div className="card-title">
                  <span>交互与叠堆</span>
                </div>

                <div className="setting-row">
                  <div>
                    <div className="s-label">框选后自动翻译</div>
                    <div className="s-desc">松开鼠标直接弹出译文，无需再点按钮</div>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    aria-label="框选后自动翻译"
                    aria-checked={config.auto_translate_on_release}
                    className={`ios-switch ${config.auto_translate_on_release ? "on" : ""}`}
                    onClick={() =>
                      setConfig({
                        ...config,
                        auto_translate_on_release: !config.auto_translate_on_release,
                      })
                    }
                  >
                    <span className="ios-switch-thumb" />
                  </button>
                </div>

                <div className="setting-row">
                  <div>
                    <div className="s-label">叠堆上限</div>
                    <div className="s-desc">右下角暂存卡片最多保留张数</div>
                  </div>
                  <SlidingSegmented<string>
                    ariaLabel="叠堆上限"
                    size="sm"
                    value={String(config.max_stack_size)}
                    onChange={(val) =>
                      setConfig({ ...config, max_stack_size: Number(val) })
                    }
                    options={[4, 6, 8, 12].map((n) => ({
                      value: String(n),
                      label: `${n} 张`,
                    }))}
                  />
                </div>

                <div className="setting-row">
                  <div>
                    <div className="s-label">导出后清空叠堆</div>
                    <div className="s-desc">复制或保存拼图后自动释放暂存队列</div>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    aria-label="导出后清空叠堆"
                    aria-checked={config.clear_stack_after_output}
                    className={`ios-switch ${config.clear_stack_after_output ? "on" : ""}`}
                    onClick={() =>
                      setConfig({
                        ...config,
                        clear_stack_after_output: !config.clear_stack_after_output,
                      })
                    }
                  >
                    <span className="ios-switch-thumb" />
                  </button>
                </div>

                <div className="setting-row">
                  <div>
                    <div className="s-label">静默启动</div>
                    <div className="s-desc">启动后直接驻留系统托盘，不弹窗打扰</div>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    aria-label="静默启动"
                    aria-checked={config.start_minimized}
                    className={`ios-switch ${config.start_minimized ? "on" : ""}`}
                    onClick={() =>
                      setConfig({
                        ...config,
                        start_minimized: !config.start_minimized,
                      })
                    }
                  >
                    <span className="ios-switch-thumb" />
                  </button>
                </div>

                <div className="setting-row">
                  <div>
                    <div className="s-label">开机自启</div>
                    <div className="s-desc">登录 {OS_LABEL} 时自动运行</div>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    aria-label="开机自启"
                    aria-checked={config.launch_at_startup}
                    className={`ios-switch ${config.launch_at_startup ? "on" : ""}`}
                    onClick={() =>
                      setConfig({
                        ...config,
                        launch_at_startup: !config.launch_at_startup,
                      })
                    }
                  >
                    <span className="ios-switch-thumb" />
                  </button>
                </div>
              </section>
            </div>
          )}

          {/* PANE 2: ENGINES & AI */}
          {pane === "engines" && (
            <div key="engines" className="v3-pane">
              <div className="v3-pane-head">
                <div>
                  <h1 className="v3-pane-title">翻译引擎</h1>
                  <p className="v3-pane-sub">默认引擎、视觉模型与快捷指令</p>
                </div>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => handleTest(config.translate_engine)}
                  disabled={testingTarget !== null}
                >
                  <Zap size={13} />
                  {testingTarget === config.translate_engine ? "测试中…" : "测试当前引擎"}
                </button>
              </div>

              <section className="card v3-card">
                <div className="card-title">
                  <span>默认翻译引擎</span>
                </div>

                <div className="engine-cards">
                  {ENGINE_OPTIONS.map((eng) => {
                    const active = config.translate_engine === eng.id;
                    return (
                      <button
                        key={eng.id}
                        type="button"
                        className={`engine-card${active ? " active" : ""}`}
                        onClick={() =>
                          setConfig({ ...config, translate_engine: eng.id })
                        }
                      >
                        <div className="engine-card-top">
                          <span className="engine-card-title">{eng.label}</span>
                          {active && <Check size={13} className="engine-card-check" />}
                        </div>
                        <div className="engine-card-desc">{eng.desc}</div>
                      </button>
                    );
                  })}
                </div>

                {config.translate_engine === "google" && (
                  <div className="v3-quiet-bar">
                    <span>Google 翻译免配置直连，可在「识别与外观」中配置网络代理。</span>
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={() => handleTest("google")}
                      disabled={testingTarget !== null}
                    >
                      <Zap size={12} /> {testingTarget === "google" ? "…" : "测试"}
                    </button>
                  </div>
                )}

                {(config.translate_engine === "deepl" || config.deepl_api_key.trim()) && (
                  <div className="field" style={{ marginTop: 4 }}>
                    <label>DeepL API Key（支持 Free / Pro）</label>
                    <div className="input-with-btn">
                      <input
                        className="input"
                        type={showDeepLKey ? "text" : "password"}
                        value={config.deepl_api_key}
                        onChange={(e) =>
                          setConfig({ ...config, deepl_api_key: e.target.value })
                        }
                        placeholder="xxxxxxxx-xxxx-...:fx"
                      />
                      <button
                        type="button"
                        className="btn"
                        onClick={() => setShowDeepLKey((v) => !v)}
                        title={showDeepLKey ? "隐藏" : "显示"}
                      >
                        {showDeepLKey ? <EyeOff size={14} /> : <Eye size={14} />}
                      </button>
                      <button
                        type="button"
                        className="btn"
                        onClick={() => handleTest("deepl")}
                        disabled={testingTarget !== null}
                      >
                        <Zap size={13} />
                        {testingTarget === "deepl" ? "…" : "测试"}
                      </button>
                    </div>
                  </div>
                )}
              </section>

              <section className="card v3-card">
                <div className="card-title">
                  <span>视觉 AI 模型</span>
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() => handleTest("ai")}
                    disabled={testingTarget !== null}
                  >
                    <Zap size={12} />
                    {testingTarget === "ai" ? "测试中…" : "测试 AI"}
                  </button>
                </div>

                <div className="shortcut-preset-row" style={{ marginTop: -2, marginBottom: 10 }}>
                  <span className="shortcut-preset-caption">预设</span>
                  {PROVIDER_PRESETS.map((p) => (
                    <button
                      key={p.id}
                      type="button"
                      className={`kbd-preset-btn${config.api_base_url === p.baseUrl ? " active" : ""}`}
                      onClick={() => changeAiEndpoint(p.baseUrl, p.defaultModel)}
                      title={p.hint}
                    >
                      {p.label}
                    </button>
                  ))}
                </div>

                <div className="field-row">
                  <div className="field">
                    <label>接口地址</label>
                    <input
                      className="input"
                      value={config.api_base_url}
                      onChange={(e) => changeAiEndpoint(e.target.value)}
                      placeholder="https://api.openai.com/v1"
                    />
                  </div>
                  <div className="field">
                    <label>模型</label>
                    <input
                      className="input"
                      value={config.model}
                      onChange={(e) =>
                        setConfig({ ...config, model: e.target.value })
                      }
                      placeholder="gpt-4o-mini"
                    />
                  </div>
                </div>

                <div className="field">
                  <label>API Key</label>
                  <div className="input-with-btn">
                    <input
                      className="input"
                      type={showAiKey ? "text" : "password"}
                      value={config.api_key}
                      onChange={(e) =>
                        setConfig({ ...config, api_key: e.target.value })
                      }
                      placeholder="sk-...（本地 Ollama 可留空）"
                    />
                    <button
                      type="button"
                      className="btn"
                      onClick={() => setShowAiKey((v) => !v)}
                      title={showAiKey ? "隐藏" : "显示"}
                    >
                      {showAiKey ? <EyeOff size={14} /> : <Eye size={14} />}
                    </button>
                  </div>
                  <small className="v3-field-hint">
                    纯文本模型自动走本地识字，多模态视觉模型直接看图。
                  </small>
                </div>

                <div className="v3-disclosure-block">
                  <button
                    type="button"
                    className="settings-disclosure-btn"
                    aria-expanded={showPromptGlossary}
                    onClick={() => setShowPromptGlossary((v) => !v)}
                  >
                    {showPromptGlossary ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
                    <span>翻译偏好与术语表</span>
                    {glossaryCount > 0 && (
                      <span className="v3-mini-badge">{glossaryCount} 条术语</span>
                    )}
                  </button>
                  {showPromptGlossary && (
                    <div className="v3-disclosure-body">
                      <div className="field">
                        <label>翻译偏好（可选）</label>
                        <textarea
                          className="textarea"
                          rows={2}
                          value={config.custom_prompt}
                          onChange={(e) =>
                            setConfig({ ...config, custom_prompt: e.target.value })
                          }
                          placeholder="如：保留代码标识符与专有名词"
                        />
                      </div>
                      <div className="field">
                        <label>术语表</label>
                        <textarea
                          className="textarea"
                          rows={3}
                          value={config.glossary}
                          onChange={(e) =>
                            setConfig({ ...config, glossary: e.target.value })
                          }
                          placeholder={"每行一条，支持 = 或 =>\nLuma = Luma\nLLM = 大语言模型"}
                        />
                      </div>
                    </div>
                  )}
                </div>
              </section>

              <AskActionsSettings
                actions={config.ask_quick_actions}
                onChange={(next) =>
                  setConfig((prev) => ({ ...prev, ask_quick_actions: next }))
                }
              />
            </div>
          )}

          {/* PANE 3: OCR, APPEARANCE & SYSTEM */}
          {pane === "system" && (
            <div key="system" className="v3-pane">
              <div className="v3-pane-head">
                <div>
                  <h1 className="v3-pane-title">识别与外观</h1>
                  <p className="v3-pane-sub">离线 OCR、浅色与夜间主题、网络代理</p>
                </div>
                <div className="v3-head-actions">
                  <button
                    type="button"
                    className="btn btn-sm"
                    disabled={saving || !savedConfig}
                    onClick={handleExportConfig}
                    title="导出设置备份 (JSON)"
                  >
                    <Download size={12} /> 导出
                  </button>
                  <button
                    type="button"
                    className="btn btn-sm"
                    disabled={saving || !savedConfig}
                    onClick={handleImportConfig}
                    title="导入设置备份 (JSON)"
                  >
                    <Upload size={12} /> 导入
                  </button>
                </div>
              </div>

              <section className="card v3-card">
                <div className="card-title">
                  <span>外观主题</span>
                </div>
                <div className="setting-row" style={{ paddingTop: 0, borderTop: "none" }}>
                  <div>
                    <div className="s-label">配色模式</div>
                    <div className="s-desc">清透浅色与深蓝夜间，实时同步至全部浮窗</div>
                  </div>
                  <SlidingSegmented<ThemeMode>
                    ariaLabel="配色模式"
                    size="sm"
                    value={config.theme}
                    onChange={applyThemeSelection}
                    options={[
                      { value: "system", label: "跟随系统" },
                      { value: "light", label: "浅色" },
                      { value: "dark", label: "夜间" },
                    ]}
                  />
                </div>
              </section>

              <LocalOcrSettings
                engineMode={config.ocr_engine}
                ocrLang={config.ocr_lang}
                downloadUrl={config.ocr_download_url}
                proxyUrl={config.proxy_url}
                onChangeEngineMode={(mode) =>
                  setConfig((prev) => ({ ...prev, ocr_engine: mode }))
                }
                onChangeOcrLang={(lang) =>
                  setConfig((prev) => ({ ...prev, ocr_lang: lang }))
                }
                onChangeDownloadUrl={(url) =>
                  setConfig((prev) => ({ ...prev, ocr_download_url: url }))
                }
                onNotice={(kind, text) => setStatus({ kind, text })}
              />

              <section className="card v3-card">
                <div className="card-title">
                  <span>网络与更新</span>
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() => handleTest("update")}
                    disabled={testingTarget !== null}
                    title="测试代理与更新源连通性"
                  >
                    <Zap size={12} />
                    {testingTarget === "update" ? "测试中…" : "测试连接"}
                  </button>
                </div>

                <div className="setting-row" style={{ paddingTop: 0, borderTop: "none" }}>
                  <div>
                    <div className="s-label">网络代理</div>
                    <div className="s-desc">用于翻译、模型下载与更新，本地地址自动直连</div>
                  </div>
                  <SlidingSegmented<"system" | "custom">
                    ariaLabel="代理模式"
                    size="sm"
                    value={proxyMode}
                    onChange={(mode) => {
                      setProxyMode(mode);
                      if (mode === "system") {
                        setConfig((prev) => ({ ...prev, proxy_url: "" }));
                        invoke<SystemProxyInfo>("get_system_proxy_info")
                          .then(setSystemProxy)
                          .catch(() => {});
                      }
                    }}
                    options={[
                      { value: "system", label: "跟随系统" },
                      { value: "custom", label: "手动" },
                    ]}
                  />
                </div>

                {proxyMode === "system" ? (
                  <div className="proxy-status-bar">
                    <span
                      className={`proxy-dot ${systemProxy?.enabled && systemProxy.proxyUrl ? "on" : ""}`}
                    />
                    <span>
                      {systemProxy?.enabled && systemProxy.proxyUrl
                        ? `系统代理：${systemProxy.proxyUrl}`
                        : "未启用系统代理，直连网络"}
                    </span>
                  </div>
                ) : (
                  <div className="field" style={{ marginTop: 10 }}>
                    <label htmlFor="custom-proxy-input">代理地址（HTTP / SOCKS5）</label>
                    <input
                      id="custom-proxy-input"
                      className="input"
                      value={config.proxy_url}
                      onChange={(e) =>
                        setConfig({ ...config, proxy_url: e.target.value })
                      }
                      placeholder="http://127.0.0.1:7890 或 socks5h://127.0.0.1:7891"
                    />
                  </div>
                )}

                <div className="v3-disclosure-block">
                  <button
                    type="button"
                    className="settings-disclosure-btn"
                    aria-expanded={showUpdateMirror}
                    onClick={() => setShowUpdateMirror((v) => !v)}
                  >
                    {showUpdateMirror ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
                    <span>自定义更新源</span>
                  </button>
                  {showUpdateMirror && (
                    <div className="v3-disclosure-body">
                      <div className="field">
                        <input
                          className="input"
                          value={config.update_feed_url}
                          onChange={(e) =>
                            setConfig({ ...config, update_feed_url: e.target.value })
                          }
                          placeholder="留空走官方源，或填写 latest.json 的 HTTPS 地址"
                        />
                        <small className="v3-field-hint">
                          不可用时自动回退官方源，安装前强制校验签名。
                        </small>
                      </div>
                    </div>
                  )}
                </div>
              </section>
            </div>
          )}

          {/* PANE 4: SIDE-BY-SIDE TRANSLATION STUDIO */}
          {pane === "text" && (
            <div key="text" className="v3-pane v3-pane-studio">
              <div className="studio-bar">
                <div className="studio-lang-cluster">
                  <span className="studio-lang-pill">自动识别</span>
                  <ArrowRight size={13} className="studio-arrow" aria-hidden="true" />
                  <select
                    className="select v3-compact-select"
                    aria-label="目标语言"
                    value={textTargetLang}
                    onChange={(e) => setTextTargetLang(e.target.value)}
                  >
                    {TARGET_LANGUAGES.map((lang) => (
                      <option key={lang} value={lang}>
                        {lang}
                      </option>
                    ))}
                  </select>
                  {glossaryCount > 0 && (
                    <span className="v3-mini-badge">术语表 · {glossaryCount}</span>
                  )}
                </div>

                <div className="studio-bar-right">
                  <SlidingSegmented<TranslateEngine>
                    ariaLabel="翻译引擎"
                    size="sm"
                    value={textEngine}
                    onChange={setTextEngine}
                    options={ENGINE_OPTIONS.map((eng) => ({
                      value: eng.id,
                      label: eng.label,
                    }))}
                  />
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() =>
                      invoke("analyze_clipboard_text").catch((err) => setTextError(String(err)))
                    }
                    title="翻译剪贴板文字并弹出浮窗"
                  >
                    <Clipboard size={12} /> 译剪贴板
                  </button>
                </div>
              </div>

              <div className="studio-grid">
                <section className="studio-sheet">
                  <div className="studio-sheet-head">
                    <strong>原文</strong>
                    {textInput && (
                      <button
                        type="button"
                        className="btn-ghost-sm"
                        onClick={() => {
                          setTextInput("");
                          setTextResult(null);
                          setTextError(null);
                        }}
                      >
                        清空
                      </button>
                    )}
                  </div>
                  <div className="studio-sheet-body">
                    <textarea
                      className="studio-textarea"
                      value={textInput}
                      onChange={(e) => setTextInput(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                          e.preventDefault();
                          handleTranslateText();
                        }
                      }}
                      placeholder={`输入或粘贴需要翻译的文本，按 ${MOD_LABEL}+Enter 立即翻译…`}
                    />
                  </div>
                  <div className="studio-sheet-foot">
                    <span className="studio-counter">{textInput.trim().length} 字</span>
                    <button
                      type="button"
                      className="btn btn-primary btn-sm"
                      onClick={handleTranslateText}
                      disabled={textBusy || !textInput.trim()}
                    >
                      <Languages size={13} />
                      {textBusy ? "翻译中…" : "翻译"}
                      <kbd className="kbd-cap kbd-cap-inv">{MOD_LABEL}↵</kbd>
                    </button>
                  </div>
                </section>

                <section className="studio-sheet studio-sheet-result">
                  <div className="studio-sheet-head">
                    <strong>
                      {textResult
                        ? `译文 · ${textResult.targetLang}`
                        : textBusy
                          ? `译文 · ${textTargetLang}`
                          : "译文"}
                    </strong>
                    {textResult && (
                      <span className="studio-latency">{textResult.elapsedMs}ms</span>
                    )}
                  </div>
                  <div className="studio-sheet-body">
                    {textError ? (
                      <div className="error-box">{textError}</div>
                    ) : textBusy && textStreamPreview && !textResult ? (
                      <div className="studio-prose">
                        {textStreamPreview}
                        <span className="stream-cursor" />
                      </div>
                    ) : textBusy && !textResult ? (
                      <div className="loading-status">
                        <span className="pulse-dot" />
                        <span>正在翻译为 {textTargetLang}…</span>
                      </div>
                    ) : textResult ? (
                      <div className="studio-prose">{textResult.translatedText}</div>
                    ) : (
                      <div className="studio-placeholder">
                        译文将在此实时呈现，支持流式输出与术语表替换。
                      </div>
                    )}
                  </div>
                  <div className="studio-sheet-foot">
                    <span className="studio-counter">
                      {textResult ? textResult.model : ENGINE_OPTIONS.find((e) => e.id === textEngine)?.label}
                    </span>
                    <div className="studio-result-actions">
                      <button
                        type="button"
                        className="btn btn-sm"
                        disabled={!textResult}
                        onClick={() => textResult && speakDirectText(textResult.translatedText)}
                        title="朗读译文"
                      >
                        <Volume2 size={12} /> 朗读
                      </button>
                      <button
                        type="button"
                        className="btn btn-sm btn-primary"
                        disabled={!textResult}
                        onClick={async () => {
                          if (!textResult) return;
                          await navigator.clipboard.writeText(textResult.translatedText);
                          setCopiedTextOut(true);
                          setTimeout(() => setCopiedTextOut(false), 1500);
                        }}
                      >
                        {copiedTextOut ? (
                          <span className="icon-morph">
                            <Check size={12} />
                          </span>
                        ) : (
                          <Copy size={12} />
                        )}
                        {copiedTextOut ? "已复制" : "复制"}
                      </button>
                    </div>
                  </div>
                </section>
              </div>
            </div>
          )}

          {/* PANE 5: HISTORY ARCHIVE */}
          {pane === "history" && (
            <div
              key="history"
              className={`v3-pane${historyViewMode !== "list" ? " v3-pane-history-stage" : ""}`}
            >
              <div className="v3-pane-head">
                <div>
                  <h1 className="v3-pane-title">历史</h1>
                  <p className="v3-pane-sub">
                    共 {history.length} 条本地记录
                    {historyViewMode !== "list" ? " · 原生滚轮实时驱动" : ""}
                  </p>
                </div>
                <div className="v3-head-actions">
                  <SlidingSegmented<HistoryViewMode>
                    ariaLabel="历史视图模式"
                    size="sm"
                    value={historyViewMode}
                    onChange={changeHistoryViewMode}
                    options={[
                      { value: "list", label: "列表" },
                      { value: "stacked", label: "层叠" },
                      { value: "coverflow", label: "3D 轮播" },
                      { value: "deck", label: "飞牌" },
                    ]}
                  />
                  {history.length > 0 && (
                    <>
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() => exportHistory("markdown")}
                        title="导出为 Markdown"
                      >
                        <Download size={12} /> MD
                      </button>
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() => exportHistory("json")}
                        title="导出为 JSON"
                      >
                        <Download size={12} /> JSON
                      </button>
                      <Popconfirm
                        message={`清空全部 ${history.length} 条历史记录？`}
                        confirmLabel="清空"
                        onConfirm={async () => {
                          try {
                            await invoke("clear_history");
                            setHistory([]);
                            setStatus({ kind: "ok", text: "已清空历史记录" });
                          } catch (err) {
                            setStatus({ kind: "err", text: String(err) });
                          }
                        }}
                      >
                        {(triggerProps) => (
                          <button
                            type="button"
                            className="btn btn-sm btn-danger"
                            {...triggerProps}
                          >
                            <Trash2 size={12} /> 清空
                          </button>
                        )}
                      </Popconfirm>
                    </>
                  )}
                </div>
              </div>

              {historyLoadError && <div className="error-box">{historyLoadError}</div>}

              <div className="history-toolbar">
                <div className="history-search-wrap">
                  <Search size={13} className="history-search-icon" />
                  <input
                    className="input history-search-input"
                    value={historyQuery}
                    onChange={(e) => setHistoryQuery(e.target.value)}
                    placeholder="搜索原文、译文或模型…"
                  />
                  {historyQuery && (
                    <button
                      type="button"
                      className="icon-btn"
                      onClick={() => setHistoryQuery("")}
                      title="清除搜索"
                    >
                      <X size={12} />
                    </button>
                  )}
                </div>

                <SlidingSegmented<"all" | "translate" | "ocr">
                  ariaLabel="历史类型筛选"
                  size="sm"
                  value={historyModeFilter}
                  onChange={setHistoryModeFilter}
                  options={[
                    { value: "all", label: "全部" },
                    { value: "translate", label: "翻译" },
                    { value: "ocr", label: "识字" },
                  ]}
                />
              </div>

              {filteredHistory.length === 0 ? (
                history.length === 0 && historyDemoMode && historyViewMode !== "list" ? (
                  <HistoryScrollStage
                    items={DEMO_HISTORY_ITEMS}
                    mode={historyViewMode as HistoryScrollMode}
                    onModeChange={(m) => changeHistoryViewMode(m)}
                    copiedId={copiedId}
                    exitingIds={exitingHistoryIds}
                    onReopen={() => undefined}
                    onCopy={copyHistory}
                    onDelete={() => undefined}
                    formatTimestamp={formatTimestamp}
                    isDemo
                    onExitDemo={() => setHistoryDemoMode(false)}
                  />
                ) : (
                  <div className="card v3-empty-state">
                    <History size={22} className="v3-empty-icon" />
                    <div>
                      {history.length === 0
                        ? `暂无记录 · 按 ${formatShortcutLabel(config.shortcut)} 开始截图翻译`
                        : "没有匹配的记录"}
                    </div>
                    {history.length === 0 && (
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() => {
                          if (historyViewMode === "list") {
                            changeHistoryViewMode("stacked");
                          }
                          setHistoryDemoMode(true);
                        }}
                      >
                        <Sparkles size={12} /> 用示例卡片体验 3 种滚轮效果
                      </button>
                    )}
                  </div>
                )
              ) : historyViewMode !== "list" ? (
                <HistoryScrollStage
                  items={filteredHistory}
                  mode={historyViewMode}
                  onModeChange={(m) => changeHistoryViewMode(m)}
                  copiedId={copiedId}
                  exitingIds={exitingHistoryIds}
                  onReopen={(id) =>
                    invoke("reopen_history_item", { id }).catch((err) =>
                      setStatus({ kind: "err", text: String(err) })
                    )
                  }
                  onCopy={copyHistory}
                  onDelete={(id) => void deleteHistoryItem(id)}
                  formatTimestamp={formatTimestamp}
                />
              ) : (
                <div className="history-cards-col">
                  {filteredHistory.map((item) => (
                    <div
                      key={item.id}
                      className={`history-item${exitingHistoryIds[item.id] ? " exiting" : ""}`}
                    >
                      <div className="history-meta">
                        <span className="history-meta-text">
                          <span className="v3-mini-badge">
                            {item.mode === "ocr" ? "识字" : `${item.sourceLang} → ${item.targetLang}`}
                          </span>
                          <span>
                            {item.model} · {item.elapsedMs}ms
                            {item.createdAtMs ? ` · ${formatTimestamp(item.createdAtMs)}` : ""}
                          </span>
                        </span>
                        <div className="history-item-actions">
                          <button
                            type="button"
                            className="btn btn-sm"
                            onClick={() =>
                              invoke("reopen_history_item", { id: item.id }).catch((err) =>
                                setStatus({ kind: "err", text: String(err) })
                              )
                            }
                            title="在浮窗中重新打开"
                          >
                            <ExternalLink size={12} /> 浮窗
                          </button>
                          <button
                            type="button"
                            className="btn btn-sm"
                            onClick={() => copyHistory(item)}
                          >
                            {copiedId === item.id ? (
                              <>
                                <span className="icon-morph">
                                  <Check size={12} />
                                </span>
                                已复制
                              </>
                            ) : (
                              <>
                                <Copy size={12} /> 复制
                              </>
                            )}
                          </button>
                          <button
                            type="button"
                            className="icon-btn history-del-btn"
                            onClick={() => void deleteHistoryItem(item.id)}
                            title="删除"
                            aria-label="删除此条记录"
                          >
                            <Trash2 size={13} />
                          </button>
                        </div>
                      </div>
                      <div className="history-body">
                        {item.imageDataUrl && (
                          <img
                            src={item.imageDataUrl}
                            alt=""
                            className="history-thumb"
                          />
                        )}
                        <div className="history-body-text">
                          <div className="history-text">{item.translatedText}</div>
                          {item.originalText && item.mode !== "ocr" && (
                            <div className="history-orig">{item.originalText}</div>
                          )}
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>

        {/* Floating Transient Toast */}
        {status.text && (
          <div
            className={`v3-toast ${status.kind}${dirty ? " above-dock" : ""}`}
            role="status"
            aria-live="polite"
          >
            <span className="v3-toast-dot" aria-hidden="true" />
            <span className="v3-toast-text">{status.text}</span>
            <button
              type="button"
              className="v3-toast-close"
              onClick={() => setStatus({ kind: "idle", text: "" })}
              aria-label="关闭提示"
            >
              <X size={12} />
            </button>
          </div>
        )}

        {/* Floating Unsaved Changes Dock (V2/V3 non-intrusive pill) */}
        <div
          className={`v3-save-dock${dirty ? " show" : ""}`}
          aria-hidden={!dirty}
        >
          <span className="v3-save-dock-label">
            <span className="v3-save-dot" aria-hidden="true" />
            有未保存的设置更改
          </span>
          <div className="v3-save-dock-actions">
            <button
              type="button"
              className="v3-dock-btn-ghost"
              disabled={saving || !dirty}
              onClick={handleDiscard}
            >
              <RotateCcw size={12} /> 放弃
            </button>
            <button
              type="button"
              className="v3-dock-btn-solid"
              disabled={saving || !dirty}
              onClick={handleSave}
            >
              {saving ? "保存中…" : "保存"}
              {!saving && <kbd className="kbd-cap kbd-cap-dock">{MOD_LABEL}+S</kbd>}
            </button>
          </div>
        </div>
      </main>

      <UpdateDialog update={appUpdate} currentVersion={appVersion} />
    </div>
  );
}
