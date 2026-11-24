import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ALT_LABEL, formatShortcutLabel, IS_MAC, MOD_LABEL, OS_LABEL } from "../utils/platform";
import { listenToWindow } from "../utils/windowEvents";
import {
  Camera,
  Check,
  ChevronDown,
  ChevronRight,
  Clipboard,
  Copy,
  Download,
  ExternalLink,
  Eye,
  EyeOff,
  History,
  Keyboard,
  Languages,
  Search,
  ScanLine,
  Settings,
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
import { LocalOcrSettings } from "../components/LocalOcrSettings";
import { Popconfirm } from "../components/Popconfirm";
import { SelectionShortcutField } from "../components/SelectionShortcutField";
import { UpdateButton } from "./UpdateButton";
import { UpdateDialog } from "./UpdateDialog";
import { useAppUpdate } from "../hooks/useAppUpdate";

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
  // Require a modifier except F1-F12 (and Space/Tab when smart single-key selection is enabled).
  const isFnKey = /^F([1-9]|1[0-2])$/.test(keyName);
  const isSingleTrigger = allowSingleKeys && (keyName === "Space" || keyName === "Tab");
  if (mods.length === 0 && !isFnKey && !isSingleTrigger) return null;
  return [...mods, keyName].join("+");
}

export function MainSettings() {
  const [tab, setTab] = useState<"settings" | "text" | "history">("settings");
  const [config, setConfig] = useState<AppConfig>(DEFAULT_CONFIG);
  const [savedConfig, setSavedConfig] = useState<AppConfig | null>(null);
  const dirty = savedConfig !== null && JSON.stringify(config) !== JSON.stringify(savedConfig);
  const startupChanged = savedConfig !== null && config.start_minimized !== savedConfig.start_minimized;
  const [history, setHistory] = useState<TranslationResult[]>([]);
  const [showAiKey, setShowAiKey] = useState(false);
  const [showDeepLKey, setShowDeepLKey] = useState(false);
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
        setStatus({ kind: "ok", text: `已录制快捷键 ${formatted}，点击保存即可生效` });
      } else if (!["Control", "Shift", "Alt", "Meta"].includes(e.key)) {
        setStatus({
          kind: "err",
          text: allowSingle
            ? `划词快捷键需使用单键 Space / Tab、F1-F12 或 ${MOD_LABEL} / ${ALT_LABEL} / Shift 组合键`
            : `快捷键需搭配 ${MOD_LABEL} / ${ALT_LABEL} / Shift 组合键或 F1-F12`,
        });
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [recordingTarget]);

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

  const handleSaveRef = useRef(handleSave);
  handleSaveRef.current = handleSave;

  useEffect(() => {
    if (tab !== "settings" || recordingTarget) return;
    const onSaveKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void handleSaveRef.current();
      }
    };
    window.addEventListener("keydown", onSaveKey);
    return () => window.removeEventListener("keydown", onSaveKey);
  }, [tab, recordingTarget]);

  const handleTest = async (target: "ai" | "google" | "deepl" | "update") => {
    setTestingTarget(target);
    setStatus({
      kind: "idle",
      text: target === "update" ? "正在测试代理与更新源连通性..." : "正在测试接口连通性...",
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
        // An incomplete URL has no origin to retain a key for.
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
    setTimeout(() => setCopiedId(null), 1400);
  };

  const deleteHistoryItem = async (id: number) => {
    try {
      const next = await invoke<TranslationResult[]>("delete_history_item", { id });
      setHistory(next);
    } catch (err) {
      setStatus({ kind: "err", text: String(err) });
    }
  };

  const exportHistory = async (format: "markdown" | "json") => {
    try {
      const ok = await invoke<boolean>("export_history", { format });
      if (ok) {
        setStatus({
          kind: "ok",
          text: `已导出历史记录 (${format === "json" ? "JSON" : "Markdown"})`,
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

  return (
    <div className="main-shell">
      <header className="topbar">
        <div className="brand">
          <div className="brand-badge">
            <ScanLine size={19} aria-hidden="true" />
          </div>
          <div>
            <div className="brand-title">
              Luma
              {appVersion && <span className="brand-tag">v{appVersion}</span>}
            </div>
            <div className="brand-sub">
              截图 {shortcutStatus?.active ? <kbd>{shortcutStatus.shortcut}</kbd> : "快捷键未启用"}
            </div>
          </div>
        </div>

        <div className="topbar-actions">
          <UpdateButton update={appUpdate} />
          <button
            type="button"
            className="btn"
            onClick={handleClipboardImage}
            title="翻译剪贴板中的图片"
          >
            <Clipboard size={14} /> 译剪贴板
          </button>

          <button
            type="button"
            className="btn btn-dark"
            onClick={handleCaptureNow}
          >
            <Camera size={14} /> 截图
          </button>
        </div>
      </header>
      <nav className="main-nav" aria-label="主窗口">
        <button
          type="button"
          className={`seg-btn ${tab === "settings" ? "active" : ""}`}
          aria-current={tab === "settings" ? "page" : undefined}
          onClick={() => setTab("settings")}
        >
          <Settings size={13} /> 设置
        </button>
        <button
          type="button"
          className={`seg-btn ${tab === "text" ? "active" : ""}`}
          aria-current={tab === "text" ? "page" : undefined}
          onClick={() => setTab("text")}
        >
          <Languages size={13} /> 翻译
        </button>
        <button
          type="button"
          className={`seg-btn ${tab === "history" ? "active" : ""}`}
          aria-current={tab === "history" ? "page" : undefined}
          onClick={() => {
            setTab("history");
            loadHistory();
          }}
        >
          <History size={13} /> 历史 ({history.length})
        </button>
      </nav>

      <main className="main-content">
        {tab === "settings" ? (
          <div className="settings-grid settings-form" aria-busy={saving}>
            {/* Section 1: Shortcut & Interaction */}
            <section className="card">
              <div className="card-title">
                <span>快捷键与常规</span>
              </div>

              <div className="field-row">
                <div className="field">
                  <label>
                    截图快捷键
                    {shortcutStatus &&
                      config.shortcut.trim() !== shortcutStatus.shortcut && (
                        <span className="unsaved-hint">
                          未保存（当前：{shortcutStatus.shortcut}）
                        </span>
                      )}
                  </label>
                  <div className="input-with-btn">
                    <input
                      className="input"
                      value={recordingTarget === "capture" ? "按下组合键（Esc 取消）" : config.shortcut}
                      readOnly={recordingTarget === "capture"}
                      onChange={(e) =>
                        setConfig({ ...config, shortcut: e.target.value })
                      }
                      placeholder={
                        IS_MAC ? "如 Option+Q" : "如 Alt+Q"
                      }
                    />
                    <button
                      type="button"
                      className={`btn ${recordingTarget === "capture" ? "btn-primary" : ""}`}
                      onClick={() =>
                        setRecordingTarget((v) => (v === "capture" ? null : "capture"))
                      }
                      title="录制截图快捷键"
                    >
                      <Keyboard size={14} />
                      {recordingTarget === "capture" ? "取消" : "录制"}
                    </button>
                  </div>
                </div>
                <div className="field">
                  <label>目标语言</label>
                  <select
                    className="select"
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

              <div className="chip-row">
                {SHORTCUT_PRESETS.map((sc) => (
                  <button
                    key={sc}
                    type="button"
                    className={`chip ${config.shortcut === sc ? "active" : ""}`}
                    onClick={() => setConfig({ ...config, shortcut: sc })}
                  >
                    {formatShortcutLabel(sc)}
                  </button>
                ))}
              </div>

              <div className="field" style={{ marginTop: 12 }}>
                <label>
                  剪贴板速译
                  {shortcutStatus &&
                    config.clipboard_shortcut.trim() !==
                      (shortcutStatus.clipboardShortcut ?? "") && (
                      <span className="unsaved-hint">
                        未保存
                      </span>
                    )}
                </label>
                <div className="input-with-btn">
                  <input
                    className="input"
                    value={
                      recordingTarget === "clipboard"
                        ? "按下组合键（Esc 取消）"
                        : config.clipboard_shortcut
                    }
                    readOnly={recordingTarget === "clipboard"}
                    onChange={(e) =>
                      setConfig({ ...config, clipboard_shortcut: e.target.value })
                    }
                    placeholder={
                      IS_MAC
                        ? "留空禁用，如 Option+W"
                        : "留空禁用，如 Alt+W"
                    }
                  />
                  <button
                    type="button"
                    className={`btn ${recordingTarget === "clipboard" ? "btn-primary" : ""}`}
                    onClick={() =>
                      setRecordingTarget((v) => (v === "clipboard" ? null : "clipboard"))
                    }
                    title="录制剪贴板速译快捷键"
                  >
                    <Keyboard size={14} />
                    {recordingTarget === "clipboard" ? "取消" : "录制"}
                  </button>
                  {config.clipboard_shortcut && (
                    <button
                      type="button"
                      className="btn"
                      onClick={() => setConfig({ ...config, clipboard_shortcut: "" })}
                      title="清除快捷键"
                    >
                      <X size={14} />
                    </button>
                  )}
                </div>
                <div className="chip-row">
                  {["", "Alt+W", "Ctrl+Alt+C", "Ctrl+Shift+Q"].map((sc) => (
                    <button
                      key={sc || "none"}
                      type="button"
                      className={`chip ${config.clipboard_shortcut === sc ? "active" : ""}`}
                      onClick={() => setConfig({ ...config, clipboard_shortcut: sc })}
                    >
                      {sc ? formatShortcutLabel(sc) : "关闭"}
                    </button>
                  ))}
                </div>
              </div>

              <SelectionShortcutField
                value={config.selection_shortcut}
                savedValue={shortcutStatus?.selectionShortcut ?? ""}
                recording={recordingTarget === "selection"}
                error={shortcutStatus?.selectionError ?? null}
                onChange={(next) =>
                  setConfig((prev) => ({ ...prev, selection_shortcut: next }))
                }
                onToggleRecord={() =>
                  setRecordingTarget((v) => (v === "selection" ? null : "selection"))
                }
              />

              {shortcutStatus && !shortcutStatus.active && shortcutStatus.error && (
                <div className="error-box" style={{ marginTop: 10 }}>
                  {shortcutStatus.error}
                </div>
              )}
              {shortcutStatus && shortcutStatus.clipboardError && (
                <div className="error-box" style={{ marginTop: 8 }}>
                  {shortcutStatus.clipboardError}
                </div>
              )}

              <div className="switch-row">
                <div>
                  <div className="switch-label">框选后自动翻译</div>
                  <div className="switch-desc">松开鼠标直接翻译</div>
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

              <div className="switch-row">
                <div>
                  <div className="switch-label">静默启动</div>
                  <div className="switch-desc">启动后最小化到托盘</div>
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

              <div className="switch-row">
                <div>
                  <div className="switch-label">开机自启</div>
                  <div className="switch-desc">登录 {OS_LABEL} 时自动运行</div>
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

              <div className="switch-row">
                <div>
                  <div className="switch-label">叠堆上限</div>
                  <div className="switch-desc">右下角暂存卡片上限</div>
                </div>
                <div className="chip-row" style={{ marginTop: 0 }}>
                  {[4, 6, 8, 12].map((n) => (
                    <button
                      key={n}
                      type="button"
                      className={`chip ${config.max_stack_size === n ? "active" : ""}`}
                      onClick={() => setConfig({ ...config, max_stack_size: n })}
                    >
                      {n} 张
                    </button>
                  ))}
                </div>
              </div>

              <div className="switch-row">
                <div>
                  <div className="switch-label">导出后清空叠堆</div>
                  <div className="switch-desc">复制或保存拼图后自动清空</div>
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

              <div className="switch-row">
                <div>
                  <div className="switch-label">外观</div>
                  <div className="switch-desc">深浅色模式，全窗口同步</div>
                </div>
                <div className="chip-row" style={{ marginTop: 0 }}>
                  {(
                    [
                      { id: "system", label: "跟随系统" },
                      { id: "light", label: "浅色" },
                      { id: "dark", label: "深色" },
                    ] as { id: ThemeMode; label: string }[]
                  ).map((t) => (
                    <button
                      key={t.id}
                      type="button"
                      className={`chip ${config.theme === t.id ? "active" : ""}`}
                      onClick={() => {
                        setConfig({ ...config, theme: t.id });
                        document.documentElement.dataset.theme = t.id;
                        invoke("preview_theme", { theme: t.id }).catch(console.error);
                      }}
                    >
                      {t.label}
                    </button>
                  ))}
                </div>
              </div>
            </section>

            {/* Section 2: Translation Engines (AI / Google / DeepL) */}
            <section className="card">
              <div className="card-title">
                <span>翻译引擎</span>
                <span className="card-subtitle">浮窗内可随时切换</span>
              </div>

              <div className="engine-cards">
                {ENGINE_OPTIONS.map((eng) => (
                  <button
                    key={eng.id}
                    type="button"
                    className={`engine-card ${
                      config.translate_engine === eng.id ? "active" : ""
                    }`}
                    onClick={() =>
                      setConfig({ ...config, translate_engine: eng.id })
                    }
                  >
                    <div className="engine-card-title">{eng.label}</div>
                    <div className="engine-card-desc">{eng.desc}</div>
                  </button>
                ))}
              </div>

              <div className="field-row">
                <div className="field">
                  <label>Google Key（可选）</label>
                  <div className="input-with-btn">
                    <input
                      className="input"
                      type="password"
                      value={config.google_api_key}
                      onChange={(e) =>
                        setConfig({ ...config, google_api_key: e.target.value })
                      }
                      placeholder="留空使用免费接口"
                    />
                    <button
                      type="button"
                      className="btn"
                      onClick={() => handleTest("google")}
                      disabled={testingTarget !== null}
                      title="测试 Google 翻译连接"
                    >
                      <Zap size={14} />
                      {testingTarget === "google" ? "…" : "测试"}
                    </button>
                  </div>
                </div>

                <div className="field">
                  <label>DeepL Key</label>
                  <div className="input-with-btn">
                    <input
                      className="input"
                      type={showDeepLKey ? "text" : "password"}
                      value={config.deepl_api_key}
                      onChange={(e) =>
                        setConfig({ ...config, deepl_api_key: e.target.value })
                      }
                      placeholder="Free (:fx) 或 Pro Key"
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
                      title="测试 DeepL 连接"
                    >
                      <Zap size={14} />
                      {testingTarget === "deepl" ? "…" : "测试"}
                    </button>
                  </div>
                </div>
              </div>
            </section>

            {/* Section 3: Vision AI Model (OCR + AI Translate + Ask AI) */}
            <section className="card">
              <div className="card-title">
                <span>视觉 AI</span>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => handleTest("ai")}
                  disabled={testingTarget !== null}
                >
                  <Zap size={13} />
                  {testingTarget === "ai" ? "测试中…" : "测试连接"}
                </button>
              </div>

              <div className="chip-row">
                {PROVIDER_PRESETS.map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    className={`chip ${
                      config.api_base_url === p.baseUrl ? "active" : ""
                    }`}
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
                  <small>
                    纯文本模型自动走本地识字，视觉模型直接看图。
                  </small>
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
                    {showAiKey ? <EyeOff size={15} /> : <Eye size={15} />}
                  </button>
                </div>
              </div>

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
            </section>

            <AskActionsSettings
              actions={config.ask_quick_actions}
              onChange={(next) =>
                setConfig((prev) => ({ ...prev, ask_quick_actions: next }))
              }
            />

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

            {/* Section 4: Network Proxy & Update Source */}
            <section className="card">
              <div className="card-title">
                <span>网络与更新</span>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => handleTest("update")}
                  disabled={testingTarget !== null}
                  title="测试代理与更新源连接"
                >
                  <Zap size={13} />
                  {testingTarget === "update" ? "测试中…" : "测试连接"}
                </button>
              </div>

              <div className="switch-row" style={{ paddingTop: 0, borderTop: "none" }}>
                <div>
                  <div className="switch-label">代理</div>
                  <div className="switch-desc">
                    用于翻译、模型下载与更新，本地地址自动直连
                  </div>
                </div>
                <div className="segmented" role="group" aria-label="代理模式">
                  <button
                    type="button"
                    className={`seg-btn ${proxyMode === "system" ? "active" : ""}`}
                    onClick={() => {
                      setProxyMode("system");
                      setConfig((prev) => ({ ...prev, proxy_url: "" }));
                      invoke<SystemProxyInfo>("get_system_proxy_info")
                        .then(setSystemProxy)
                        .catch(() => {});
                    }}
                  >
                    跟随系统
                  </button>
                  <button
                    type="button"
                    className={`seg-btn ${proxyMode === "custom" ? "active" : ""}`}
                    onClick={() => setProxyMode("custom")}
                  >
                    手动
                  </button>
                </div>
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

              <div style={{ marginTop: 12 }}>
                <button
                  type="button"
                  className="settings-disclosure-btn"
                  aria-expanded={showUpdateMirror}
                  onClick={() => setShowUpdateMirror((v) => !v)}
                >
                  {showUpdateMirror ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
                  自定义更新源
                </button>
                {showUpdateMirror && (
                  <div className="field" style={{ marginTop: 8 }}>
                    <input
                      className="input"
                      value={config.update_feed_url}
                      onChange={(e) =>
                        setConfig({ ...config, update_feed_url: e.target.value })
                      }
                      placeholder="留空走官方源，或填写 latest.json 的 HTTPS 地址"
                    />
                    <small>
                      不可用时自动回退官方源，安装前强制校验签名。
                    </small>
                  </div>
                )}
              </div>
            </section>

          </div>
        ) : tab === "text" ? (
          <div className="settings-grid">
            <section className="card">
              <div className="card-title">
                <span>文本翻译</span>
                <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
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

              <div className="text-workbench-controls">
                <div className="mini-engine-row">
                  {ENGINE_OPTIONS.map((eng) => (
                    <button
                      key={eng.id}
                      type="button"
                      className={`mini-chip ${textEngine === eng.id ? "active" : ""}`}
                      onClick={() => setTextEngine(eng.id)}
                    >
                      {eng.label}
                    </button>
                  ))}
                </div>
                <select
                  className="select"
                  style={{ width: 140, padding: "5px 8px" }}
                  value={textTargetLang}
                  onChange={(e) => setTextTargetLang(e.target.value)}
                >
                  {TARGET_LANGUAGES.map((lang) => (
                    <option key={lang} value={lang}>
                      {lang}
                    </option>
                  ))}
                </select>
              </div>

              <div className="field" style={{ marginTop: 10 }}>
                <textarea
                  className="textarea"
                  rows={5}
                  value={textInput}
                  onChange={(e) => setTextInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                      e.preventDefault();
                      handleTranslateText();
                    }
                  }}
                  placeholder={`输入或粘贴文本，${MOD_LABEL}+Enter 翻译…`}
                />
              </div>

              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 8 }}>
                <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
                  {textInput.trim().length} 字 · {MOD_LABEL}+Enter
                </span>
                <div style={{ display: "flex", gap: 8 }}>
                  {textInput && (
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={() => {
                        setTextInput("");
                        setTextResult(null);
                        setTextError(null);
                      }}
                    >
                      清空
                    </button>
                  )}
                  <button
                    type="button"
                    className="btn btn-primary"
                    onClick={handleTranslateText}
                    disabled={textBusy || !textInput.trim()}
                  >
                    <Languages size={14} /> {textBusy ? "翻译中…" : "翻译"}
                  </button>
                </div>
              </div>
            </section>

            {textError && <div className="error-box">{textError}</div>}

            {textBusy && textStreamPreview && !textResult && (
              <section className="card">
                <div className="card-title">
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
                    <span className="pulse-dot" />
                    译文 · {textTargetLang}
                  </span>
                </div>
                <div className="result-translated">
                  {textStreamPreview}
                  <span className="stream-cursor" />
                </div>
              </section>
            )}

            {textResult && (
              <section className="card">
                <div className="card-title">
                  <span>
                    译文 · {textResult.targetLang} · {textResult.elapsedMs}ms
                  </span>
                  <div style={{ display: "flex", gap: 6 }}>
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={() => speakDirectText(textResult.translatedText)}
                      title="朗读"
                    >
                      <Volume2 size={12} /> 朗读
                    </button>
                    <button
                      type="button"
                      className="btn btn-sm btn-primary"
                      onClick={async () => {
                        await navigator.clipboard.writeText(textResult.translatedText);
                        setCopiedTextOut(true);
                        setTimeout(() => setCopiedTextOut(false), 1500);
                      }}
                    >
                      {copiedTextOut ? <Check size={12} /> : <Copy size={12} />}
                      {copiedTextOut ? "已复制" : "复制"}
                    </button>
                  </div>
                </div>
                <div className="result-translated">{textResult.translatedText}</div>
              </section>
            )}
          </div>
        ) : (
          <div className="history-list">
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

              <div className="segmented">
                {(
                  [
                    { id: "all", label: "全部" },
                    { id: "translate", label: "翻译" },
                    { id: "ocr", label: "识字" },
                  ] as const
                ).map((f) => (
                  <button
                    key={f.id}
                    type="button"
                    className={`seg-btn ${historyModeFilter === f.id ? "active" : ""}`}
                    onClick={() => setHistoryModeFilter(f.id)}
                  >
                    {f.label}
                  </button>
                ))}
              </div>
            </div>

            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
                gap: 8,
                flexWrap: "wrap",
              }}
            >
              <span style={{ fontSize: 12.5, color: "var(--text-muted)" }}>
                共 {history.length} 条
                {filteredHistory.length !== history.length ? ` · 匹配 ${filteredHistory.length} 条` : ""}
              </span>
              {history.length > 0 && (
                <div style={{ display: "flex", gap: 6 }}>
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() => exportHistory("markdown")}
                  >
                    <Download size={12} /> 导出 MD
                  </button>
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() => exportHistory("json")}
                  >
                    <Download size={12} /> 导出 JSON
                  </button>
                  <Popconfirm
                    message="清空全部历史记录？"
                    confirmLabel="清空"
                    onConfirm={async () => {
                      await invoke("clear_history");
                      setHistory([]);
                    }}
                  >
                    {(triggerProps) => (
                      <button type="button" className="btn btn-sm" {...triggerProps}>
                        <Trash2 size={12} /> 清空
                      </button>
                    )}
                  </Popconfirm>
                </div>
              )}
            </div>

            {filteredHistory.length === 0 ? (
              <div
                className="card"
                style={{
                  textAlign: "center",
                  color: "var(--text-muted)",
                  padding: 36,
                  fontSize: 13,
                }}
              >
                {history.length === 0
                  ? `暂无记录，按 ${formatShortcutLabel(config.shortcut || "Alt+Q")} 截图`
                  : "没有匹配的记录"}
              </div>
            ) : (
              filteredHistory.map((item) => (
                <div key={item.id} className="history-item">
                  <div className="history-meta">
                    <span>
                      {item.mode === "ocr"
                        ? `识字 (${item.sourceLang})`
                        : `${item.sourceLang} → ${item.targetLang}`}{" "}
                      · {item.model} · {item.elapsedMs}ms
                      {item.createdAtMs ? ` · ${formatTimestamp(item.createdAtMs)}` : ""}
                    </span>
                    <div style={{ display: "flex", gap: 6 }}>
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() =>
                          invoke("reopen_history_item", { id: item.id }).catch((err) =>
                            setStatus({ kind: "err", text: String(err) })
                          )
                        }
                        title="在浮窗中打开"
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
                            <Check size={12} /> 已复制
                          </>
                        ) : (
                          <>
                            <Copy size={12} /> 复制
                          </>
                        )}
                      </button>
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() => deleteHistoryItem(item.id)}
                        title="删除"
                      >
                        <Trash2 size={12} />
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
                    <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: 6 }}>
                      <div className="history-text">{item.translatedText}</div>
                      {item.originalText && item.mode !== "ocr" && (
                        <div className="history-orig">{item.originalText}</div>
                      )}
                    </div>
                  </div>
                </div>
              ))
            )}
          </div>
        )}
      </main>
      {(tab === "settings" || status.text) && (
        <footer className="footer-bar">
          <div className={`status-msg ${status.kind}`} role="status">
            {status.text && <div>{status.text}</div>}
            {dirty && <div className="draft-status">有未保存修改</div>}
            {!status.text && !dirty && tab === "settings" && (
              <div>
                {savedConfig
                  ? "已保存"
                  : configLoadError ?? "正在读取设置…"}
              </div>
            )}
            {historyLoadError && tab === "history" && (
              <div style={{ color: "var(--danger)" }}>{historyLoadError}</div>
            )}
          </div>
          {tab === "settings" && (
            <div className="footer-actions">
              <button type="button" className="btn btn-sm" disabled={saving || !savedConfig}
                onClick={handleExportConfig} title="导出设置">
                <Download size={13} /> 导出
              </button>
              <button type="button" className="btn btn-sm" disabled={saving || !savedConfig}
                onClick={handleImportConfig} title="导入设置">
                <Upload size={13} /> 导入
              </button>
              <button type="button" className="btn btn-primary" onClick={handleSave} disabled={saving || !dirty}>
                {saving ? "保存中…" : "保存"}
              </button>
            </div>
          )}
        </footer>
      )}
      <UpdateDialog update={appUpdate} currentVersion={appVersion} />
    </div>
  );
}
