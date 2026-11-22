import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { listenToWindow as listen } from "../utils/windowEvents";
import { MOD_LABEL } from "../utils/platform";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  Download,
  Edit3,
  Image as ImageIcon,
  Languages,
  Loader2,
  Pin,
  RotateCw,
  ScanText,
  Send,
  Sparkles,
  Volume2,
  VolumeX,
  WrapText,
  X,
} from "lucide-react";
import {
  ENGINE_OPTIONS,
  ResultMode,
  ResultViewState,
  StreamPreviewEvent,
  TARGET_LANGUAGES,
  TranslateEngine,
} from "../types";
import { AskQuickActionsBar } from "../components/AskQuickActionsBar";

const OPACITY_LEVELS = [1, 0.78, 0.52];

function unwrapParagraphLines(text: string): string {
  const normalized = text.replace(/\r\n/g, "\n").trim();
  if (!normalized) return "";
  return normalized
    .split(/\n{2,}/)
    .map((para) => {
      const lines = para
        .split("\n")
        .map((l) => l.trim())
        .filter(Boolean);
      let joined = "";
      for (const line of lines) {
        if (!joined) {
          joined = line;
          continue;
        }
        const prevChar = joined.slice(-1);
        const nextChar = line.slice(0, 1);
        const isCjkJoin =
          /[一-鿿぀-ヿ가-힯]/.test(prevChar) &&
          /[一-鿿぀-ヿ가-힯]/.test(nextChar);
        if (prevChar === "-" && /[a-zA-Z]/.test(nextChar)) {
          joined = joined.slice(0, -1) + line;
        } else {
          joined += (isCjkJoin ? "" : " ") + line;
        }
      }
      return joined;
    })
    .join("\n\n");
}

function guessSpeechLang(label: string): string {
  if (label.includes("中文") || label.includes("Chinese")) return "zh-CN";
  if (label.includes("English") || label.includes("英")) return "en-US";
  if (label.includes("日本") || label.includes("Japanese")) return "ja-JP";
  if (label.includes("한국") || label.includes("韩")) return "ko-KR";
  if (label.includes("Français") || label.includes("法")) return "fr-FR";
  if (label.includes("Deutsch") || label.includes("德")) return "de-DE";
  if (label.includes("Español") || label.includes("西")) return "es-ES";
  return "";
}

export function ResultPopup() {
  const [state, setState] = useState<ResultViewState>({ status: "idle" });
  const [pinned, setPinned] = useState(false);
  const [showThumb, setShowThumb] = useState(false);
  const [showOriginal, setShowOriginal] = useState(false);
  const [mergeLines, setMergeLines] = useState(false);
  const [editingOrig, setEditingOrig] = useState(false);
  const [origDraft, setOrigDraft] = useState("");
  const [speakingKey, setSpeakingKey] = useState<string | null>(null);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [askInput, setAskInput] = useState("");
  const [pinOpacityIdx, setPinOpacityIdx] = useState(0);
  const [activeEngine, setActiveEngine] = useState<TranslateEngine>("ai");
  const [activeTargetLang, setActiveTargetLang] = useState<string>("简体中文");
  const [pinScale, setPinScale] = useState(1);
  const [streamPreview, setStreamPreview] = useState<StreamPreviewEvent | null>(null);
  const [enterSeq, setEnterSeq] = useState(0);

  const threadEndRef = useRef<HTMLDivElement | null>(null);
  const headerRef = useRef<HTMLDivElement | null>(null);
  const footerRef = useRef<HTMLDivElement | null>(null);
  const bodyInnerRef = useRef<HTMLDivElement | null>(null);
  const lastSizeRef = useRef<{ width: number; height: number }>({ width: 0, height: 0 });
  const wasVisibleRef = useRef(false);
  const lastJobIdRef = useRef<number | null>(null);
  const lastResultIdRef = useRef<number | null>(null);

  const stopSpeech = useCallback(() => {
    if (typeof window !== "undefined" && "speechSynthesis" in window) {
      window.speechSynthesis.cancel();
    }
    setSpeakingKey(null);
  }, []);

  const toggleSpeak = useCallback(
    (text: string, key: string, langHint = "") => {
      if (typeof window === "undefined" || !("speechSynthesis" in window)) return;
      if (speakingKey === key) {
        stopSpeech();
        return;
      }
      const trimmed = text.trim();
      if (!trimmed) return;
      window.speechSynthesis.cancel();
      const utter = new SpeechSynthesisUtterance(trimmed);
      const lang = guessSpeechLang(langHint);
      if (lang) utter.lang = lang;
      utter.onend = () => setSpeakingKey((cur) => (cur === key ? null : cur));
      utter.onerror = () => setSpeakingKey((cur) => (cur === key ? null : cur));
      setSpeakingKey(key);
      window.speechSynthesis.speak(utter);
    },
    [speakingKey, stopSpeech]
  );

  const handleClose = useCallback(async () => {
    stopSpeech();
    setAskInput("");
    setPinOpacityIdx(0);
    setPinScale(1);
    setMergeLines(false);
    setEditingOrig(false);
    setPinned(false);
    await invoke("hide_result_window");
  }, [stopSpeech]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let unlistenStream: (() => void) | undefined;

    const applyViewState = (next: ResultViewState) => {
      const isCardVisible = next.status !== "idle" && next.status !== "pin";
      const prevVisible = wasVisibleRef.current;
      wasVisibleRef.current = isCardVisible;

      if (!isCardVisible) {
        lastJobIdRef.current = null;
        lastSizeRef.current = { width: 0, height: 0 };
        setPinned(false);
      } else if (
        !prevVisible ||
        (next.status === "loading" && next.jobId !== lastJobIdRef.current)
      ) {
        setEnterSeq((n) => n + 1);
      }
      if ("jobId" in next) {
        lastJobIdRef.current = next.jobId;
      }

      setState(next);
      setStreamPreview(null);
      setCopiedKey(null);
      stopSpeech();

      if (next.status === "loading") {
        setShowOriginal(false);
        setMergeLines(false);
        setEditingOrig(false);
        setActiveEngine(next.engine);
        setActiveTargetLang(next.targetLang);
      } else if (next.status === "done") {
        if (lastResultIdRef.current !== next.result.id) {
          lastResultIdRef.current = next.result.id;
          setShowOriginal(false);
          setMergeLines(false);
        }
        setEditingOrig(false);
        setActiveEngine(next.result.engine);
        if (next.result.mode === "translate") {
          setActiveTargetLang(next.result.targetLang);
        }
      } else if (next.status === "pin") {
        setPinScale(1);
        setPinOpacityIdx(0);
      }
    };

    invoke<ResultViewState>("get_latest_result_state")
      .then((initial) => {
        applyViewState(initial);
      })
      .catch(console.error);

    listen<ResultViewState>("result:update", (event) => {
      applyViewState(event.payload);
    }).then((fn) => {
      unlisten = fn;
    });

    listen<StreamPreviewEvent>("result:stream", (event) => {
      setStreamPreview(event.payload);
    }).then((fn) => {
      unlistenStream = fn;
    });

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        handleClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);

    return () => {
      stopSpeech();
      window.removeEventListener("keydown", onKeyDown);
      if (unlisten) unlisten();
      if (unlistenStream) unlistenStream();
    };
  }, [handleClose, stopSpeech]);

  useEffect(() => {
    if (state.status === "ask") {
      threadEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [state, streamPreview]);

  useLayoutEffect(() => {
    if (state.status === "idle" || state.status === "pin") return;

    let rafId = 0;
    const syncSize = () => {
      if (state.status === "ask") {
        const target = { width: 456, height: 470 };
        const prev = lastSizeRef.current;
        if (Math.abs(target.width - prev.width) >= 4 || Math.abs(target.height - prev.height) >= 4) {
          lastSizeRef.current = target;
          invoke("resize_result_window", target).catch(() => {});
        }
        return;
      }

      const headerH = headerRef.current?.offsetHeight ?? 40;
      const footerH = footerRef.current?.offsetHeight ?? 0;
      const innerH = bodyInnerRef.current?.getBoundingClientRect().height ?? 80;
      const desiredH = Math.max(156, Math.min(440, Math.ceil(headerH + innerH + footerH + 34)));
      const desiredW = 456;

      const prev = lastSizeRef.current;
      if (Math.abs(desiredW - prev.width) >= 4 || Math.abs(desiredH - prev.height) >= 4) {
        lastSizeRef.current = { width: desiredW, height: desiredH };
        invoke("resize_result_window", { width: desiredW, height: desiredH }).catch(() => {});
      }
    };

    const scheduleSync = () => {
      cancelAnimationFrame(rafId);
      rafId = requestAnimationFrame(syncSize);
    };

    scheduleSync();
    const ro =
      typeof ResizeObserver !== "undefined" ? new ResizeObserver(scheduleSync) : null;
    if (ro && bodyInnerRef.current) {
      ro.observe(bodyInnerRef.current);
    }

    return () => {
      cancelAnimationFrame(rafId);
      ro?.disconnect();
    };
  }, [state, streamPreview, showOriginal, editingOrig, showThumb]);

  const toggleAlwaysOnTop = async () => {
    const next = !pinned;
    setPinned(next);
    await getCurrentWebviewWindow().setAlwaysOnTop(next);
  };

  const markCopied = (key: string) => {
    setCopiedKey(key);
    setTimeout(() => setCopiedKey(null), 1600);
  };

  const copyText = async (text: string, key: string) => {
    try {
      await navigator.clipboard.writeText(text);
      markCopied(key);
    } catch (err) {
      console.error("复制失败:", err);
    }
  };

  const copyCropImage = async () => {
    try {
      await invoke("copy_last_crop_image");
      markCopied("img");
    } catch (err) {
      console.error(err);
    }
  };

  const saveCropImage = async () => {
    try {
      await invoke("save_last_crop_image");
    } catch (err) {
      console.error(err);
    }
  };

  const switchMode = async (
    mode: ResultMode,
    engineOverride?: TranslateEngine,
    targetLangOverride?: string
  ) => {
    try {
      await invoke("switch_result_mode", {
        mode,
        engineOverride: engineOverride ?? null,
        targetLangOverride: targetLangOverride ?? null,
      });
    } catch (err) {
      console.error(err);
    }
  };

  const handleAskSend = async (promptText?: string) => {
    const question = (promptText ?? askInput).trim();
    if (!question) return;
    if (!promptText) setAskInput("");
    try {
      await invoke("ask_ai_follow_up", { question });
    } catch (err) {
      console.error(err);
    }
  };

  const handlePinWheel = async (e: React.WheelEvent<HTMLDivElement>) => {
    if (state.status !== "pin") return;
    e.preventDefault();
    const factor = e.deltaY < 0 ? 1.1 : 0.9;
    const nextScale = Math.max(0.35, Math.min(2.4, pinScale * factor));
    setPinScale(nextScale);

    const sf = Math.max(state.scaleFactor || 1, 1);
    const baseW = Math.max(180, Math.min(920, state.pixelWidth / sf));
    const baseH = Math.max(120, Math.min(720, state.pixelHeight / sf));
    const w = Math.max(160, Math.min(1150, Math.round(baseW * nextScale) + 12));
    const h = Math.max(100, Math.min(860, Math.round(baseH * nextScale) + 12));
    await getCurrentWebviewWindow().setSize(new LogicalSize(w, h));
  };

  if (state.status === "idle") {
    return null;
  }

  // ==================== Sticky Desktop Pin Mode ====================
  if (state.status === "pin") {
    const opacity = OPACITY_LEVELS[pinOpacityIdx];
    return (
      <div
        className="result-shell"
        onWheel={handlePinWheel}
        onDoubleClick={(e) => {
          if ((e.target as HTMLElement).closest(".pin-hud, .pin-close-corner")) return;
          handleClose();
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          handleClose();
        }}
      >
        <div className="pin-card" style={{ opacity }}>
          <img
            src={state.imageDataUrl}
            alt="桌面贴图"
            className="pin-img"
            data-tauri-drag-region
            draggable={false}
          />
          <button
            type="button"
            className="pin-close-corner"
            onClick={handleClose}
            title="关闭贴图（也可双击、右键或按 Esc）"
            aria-label="关闭贴图"
          >
            <X size={13} />
          </button>
          <div className="pin-hud">
            <button
              type="button"
              className="hud-icon-btn"
              onClick={() => switchMode("translate")}
              title="翻译此图"
            >
              <Languages size={13} /> 翻译
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={() => switchMode("ocr")}
              title="识别文字"
            >
              <ScanText size={13} /> 识字
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={() => switchMode("ask")}
              title="问 AI"
            >
              <Sparkles size={13} /> 问 AI
            </button>
            <div className="hud-sep" />
            <button
              type="button"
              className="hud-icon-btn"
              onClick={copyCropImage}
              title="复制截图"
            >
              {copiedKey === "img" ? <Check size={13} /> : <Copy size={13} />}
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={saveCropImage}
              title="保存 PNG"
            >
              <Download size={13} />
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={() => setPinOpacityIdx((i) => (i + 1) % OPACITY_LEVELS.length)}
              title="切换透明度"
            >
              {Math.round(opacity * 100)}%
            </button>
            <button
              type="button"
              className="hud-icon-btn pin-close-inline"
              onClick={handleClose}
              title="关闭贴图 (Esc)"
            >
              <X size={14} />
            </button>
          </div>
        </div>
      </div>
    );
  }

  const currentMode: ResultMode =
    state.status === "ask"
      ? "ask"
      : state.status === "done"
        ? state.result.mode
        : state.status === "loading" || state.status === "error"
          ? state.mode
          : "translate";

  const thumbUrl =
    state.status === "loading"
      ? state.imageDataUrl
      : state.status === "done"
        ? state.result.imageDataUrl
        : state.status === "ask"
          ? state.imageDataUrl
          : state.status === "error"
            ? state.imageDataUrl
            : "";

  const hasSourceImage = state.status === "done" ? state.hasSourceImage : Boolean(thumbUrl);
  const hasSourceText =
    state.status === "done"
      ? Boolean(state.result.originalText.trim())
      : state.status === "ask"
        ? Boolean(state.sourceTextPreview?.trim())
        : false;

  const doneOcr =
    state.status === "done" && state.result.mode === "ocr" ? state.result : null;
  const doneTranslate =
    state.status === "done" && state.result.mode === "translate" ? state.result : null;
  const displayOcr = doneOcr
    ? mergeLines
      ? unwrapParagraphLines(doneOcr.translatedText)
      : doneOcr.translatedText
    : "";
  const displayOriginal = doneTranslate
    ? mergeLines
      ? unwrapParagraphLines(doneTranslate.originalText)
      : doneTranslate.originalText
    : "";
  const hasMergeableOcrLines = doneOcr
    ? unwrapParagraphLines(doneOcr.translatedText) !==
      doneOcr.translatedText.replace(/\r\n/g, "\n").trim()
    : false;
  const hasMergeableOrigLines = doneTranslate
    ? unwrapParagraphLines(doneTranslate.originalText) !==
      doneTranslate.originalText.replace(/\r\n/g, "\n").trim()
    : false;

  return (
    <div className="result-shell">
      <div
        className={`result-card ${enterSeq > 0 ? "is-entering" : ""}`}
        data-enter-seq={enterSeq}
      >
        <div className="result-header" ref={headerRef} data-tauri-drag-region>
          <div className="mode-pills">
            <button
              type="button"
              className={`mode-pill ${currentMode === "translate" ? "active" : ""}`}
              onClick={() => switchMode("translate", activeEngine, activeTargetLang)}
            >
              <Languages size={12} /> 翻译
            </button>
            {!hasSourceImage && currentMode === "ocr" && (
              <button
                type="button"
                className="mode-pill active"
                onClick={() => undefined}
                title="历史 OCR 识别结果"
              >
                <ScanText size={12} /> 识字
              </button>
            )}
            {hasSourceImage && (
              <button
                type="button"
                className={`mode-pill ${currentMode === "ocr" ? "active" : ""}`}
                onClick={() => switchMode("ocr")}
              >
                <ScanText size={12} /> 识字
              </button>
            )}
            {(hasSourceImage || hasSourceText) && (
              <button
                type="button"
                className={`mode-pill ${currentMode === "ask" ? "active" : ""}`}
                onClick={() => switchMode("ask")}
              >
                <Sparkles size={12} /> 问 AI
              </button>
            )}
            {hasSourceImage && (
              <button
                type="button"
                className="mode-pill"
                onClick={() => switchMode("pin")}
                title="转为桌面贴图"
              >
                <Pin size={12} /> 贴图
              </button>
            )}
          </div>

          <div className="icon-actions">
            {thumbUrl && (
              <button
                type="button"
                className={`icon-btn ${showThumb ? "active" : ""}`}
                onClick={() => setShowThumb((v) => !v)}
                title="查看/隐藏截图缩略图"
              >
                <ImageIcon size={13} />
              </button>
            )}
            {hasSourceImage && (
              <button
                type="button"
                className="icon-btn"
                onClick={copyCropImage}
                title="复制截图图片"
              >
                {copiedKey === "img" ? <Check size={13} /> : <Copy size={13} />}
              </button>
            )}
            <button
              type="button"
              className={`icon-btn ${pinned ? "active" : ""}`}
              onClick={toggleAlwaysOnTop}
              title={pinned ? "取消置顶" : "窗口置顶"}
            >
              <Pin size={13} />
            </button>
            <button
              type="button"
              className="icon-btn"
              onClick={handleClose}
              title="关闭 (Esc)"
            >
              <X size={14} />
            </button>
          </div>
        </div>

        <div className="result-body">
          <div className="result-body-inner" ref={bodyInnerRef}>
            {showThumb && thumbUrl && (
              <img src={thumbUrl} alt="选区截图" className="result-thumb" />
            )}

            {state.status === "loading" && (() => {
              const activeStream =
                streamPreview &&
                streamPreview.jobId === state.jobId &&
                (streamPreview.text || streamPreview.originalText)
                  ? streamPreview
                  : null;
              return (
                <div className="result-section">
                  <div className="loading-status">
                    <span className="pulse-dot" />
                    <span>
                      {activeStream
                        ? state.mode === "ocr"
                          ? "正在识别文字..."
                          : `正在翻译为 ${state.targetLang}...`
                        : state.mode === "ocr"
                          ? "正在识别图中文字..."
                          : state.engine === "google"
                            ? `正在调用 Google 翻译 (${state.targetLang})...`
                            : state.engine === "deepl"
                              ? `正在调用 DeepL (${state.targetLang})...`
                              : `正在翻译为 ${state.targetLang}...`}
                    </span>
                  </div>
                  {activeStream ? (
                    activeStream.text ? (
                      <div className="result-translated">
                        {activeStream.text}
                        <span className="stream-cursor" />
                      </div>
                    ) : (
                      <div className="skeleton-box">
                        <div className="skeleton-line" style={{ width: "86%" }} />
                        <div className="skeleton-line" style={{ width: "68%" }} />
                      </div>
                    )
                  ) : (
                    <div className="skeleton-box">
                      <div className="skeleton-line" style={{ width: "86%" }} />
                      <div className="skeleton-line" style={{ width: "64%" }} />
                    </div>
                  )}
                </div>
              );
            })()}

            {state.status === "error" && (
              <div className="error-box">
                <div>{state.message}</div>
                <div style={{ marginTop: 10, display: "flex", gap: 8 }}>
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() =>
                      invoke("retry_translation", {
                        engineOverride: activeEngine,
                        targetLangOverride: activeTargetLang,
                      })
                    }
                  >
                    <RotateCw size={12} /> 重试
                  </button>
                </div>
              </div>
            )}

            {doneOcr && (
              <div className="result-section">
                <div className="result-translated">
                  {displayOcr || "（未检测到文字）"}
                </div>
              </div>
            )}

            {doneTranslate && (
              <>
                <div className="result-section">
                  <div className="result-translated">
                    {doneTranslate.blocks && doneTranslate.blocks.length > 1 ? (
                      <div className="bilingual-pair-list">
                        {doneTranslate.blocks.map((blk, bIdx) => (
                          <div key={bIdx} className="bilingual-pair-row">
                            <span className="bilingual-pair-idx">{bIdx + 1}</span>
                            <div className="bilingual-pair-body">
                              <div className="bilingual-pair-trans">{blk.translatedText}</div>
                              {blk.originalText && (
                                <div className="bilingual-pair-orig">{blk.originalText}</div>
                              )}
                            </div>
                          </div>
                        ))}
                      </div>
                    ) : (
                      doneTranslate.translatedText || "（未返回译文）"
                    )}
                  </div>
                </div>

                {showOriginal && doneTranslate.originalText && (
                  <div className="result-orig-drawer">
                    <div className="result-section-head">
                      <span>
                        原文 · {doneTranslate.sourceLang} · {doneTranslate.model}
                      </span>
                      <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
                        <button
                          type="button"
                          className={`icon-btn ${editingOrig ? "active" : ""}`}
                          onClick={() => {
                            if (editingOrig) {
                              setEditingOrig(false);
                            } else {
                              setOrigDraft(displayOriginal);
                              setEditingOrig(true);
                            }
                          }}
                          title="编辑原文后重译"
                        >
                          <Edit3 size={12} />
                        </button>
                        {hasMergeableOrigLines && (
                          <>
                            <button
                              type="button"
                              className={`mini-chip ${mergeLines ? "active" : ""}`}
                              onClick={() => setMergeLines((v) => !v)}
                              title="去除 PDF / 截图里句子中间的多余单行换行（保留段落空行）"
                            >
                              <WrapText size={11} style={{ verticalAlign: "-1px", marginRight: 2 }} />
                              合并换行
                            </button>
                            {mergeLines && !editingOrig && (
                              <button
                                type="button"
                                className="btn btn-sm"
                                onClick={() =>
                                  invoke("retranslate_custom_text", {
                                    text: displayOriginal,
                                    engineOverride: activeEngine,
                                    targetLangOverride: activeTargetLang,
                                  }).catch(console.error)
                                }
                                title="按合并换行后的原文重新翻译"
                              >
                                <RotateCw size={11} /> 重译
                              </button>
                            )}
                          </>
                        )}
                        <button
                          type="button"
                          className="icon-btn"
                          onClick={() => copyText(displayOriginal, "orig")}
                          title="复制原文"
                        >
                          {copiedKey === "orig" ? <Check size={12} /> : <Copy size={12} />}
                        </button>
                      </div>
                    </div>
                    {editingOrig ? (
                      <div className="result-edit-wrap">
                        <textarea
                          className="result-edit-textarea"
                          rows={3}
                          value={origDraft}
                          onChange={(e) => setOrigDraft(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                              e.preventDefault();
                              if (origDraft.trim()) {
                                invoke("retranslate_custom_text", {
                                  text: origDraft,
                                  engineOverride: activeEngine,
                                  targetLangOverride: activeTargetLang,
                                }).catch(console.error);
                              }
                            }
                          }}
                          placeholder={`修改原文，按 ${MOD_LABEL}+Enter 重译...`}
                        />
                        <div style={{ display: "flex", justifyContent: "flex-end", gap: 6 }}>
                          <button
                            type="button"
                            className="btn btn-sm"
                            onClick={() => setEditingOrig(false)}
                          >
                            取消
                          </button>
                          <button
                            type="button"
                            className="btn btn-sm btn-primary"
                            disabled={!origDraft.trim()}
                            onClick={() =>
                              invoke("retranslate_custom_text", {
                                text: origDraft,
                                engineOverride: activeEngine,
                                targetLangOverride: activeTargetLang,
                              }).catch(console.error)
                            }
                          >
                            <RotateCw size={11} /> 重译
                          </button>
                        </div>
                      </div>
                    ) : (
                      <div className="result-original">{displayOriginal}</div>
                    )}
                  </div>
                )}
              </>
            )}

            {state.status === "ask" && (
              <>
                {state.imageDataUrl ? (
                  <div className="ask-preview-bar">
                    <span>
                      当前选区 · {state.pixelWidth}×{state.pixelHeight}px
                    </span>
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={() => setShowThumb((v) => !v)}
                    >
                      {showThumb ? "收起截图" : "查看截图"}
                    </button>
                  </div>
                ) : (
                  state.sourceTextPreview && (
                    <div className="ask-preview-bar">
                      <span
                        title={state.sourceTextPreview}
                        style={{
                          overflow: "hidden",
                          textOverflow: "ellipsis",
                          whiteSpace: "nowrap",
                          maxWidth: "100%",
                        }}
                      >
                        当前文本 · {state.sourceTextPreview}
                      </span>
                    </div>
                  )
                )}

                <div className="ask-thread">
                  {state.messages.map((msg, i) => (
                    <div key={i} className={`ask-bubble ${msg.role}`}>
                      <div>{msg.content}</div>
                      {msg.role === "assistant" && (
                        <div style={{ marginTop: 6, display: "flex", justifyContent: "flex-end" }}>
                          <button
                            type="button"
                            className="btn btn-sm"
                            onClick={() => copyText(msg.content, `ask-${i}`)}
                          >
                            {copiedKey === `ask-${i}` ? (
                              <>
                                <Check size={11} /> 已复制
                              </>
                            ) : (
                              <>
                                <Copy size={11} /> 复制
                              </>
                            )}
                          </button>
                        </div>
                      )}
                    </div>
                  ))}

                  {state.loading &&
                    (streamPreview &&
                    streamPreview.jobId === state.jobId &&
                    streamPreview.kind === "ask" &&
                    streamPreview.text ? (
                      <div className="ask-bubble assistant streaming">
                        <div>
                          {streamPreview.text}
                          <span className="stream-cursor" />
                        </div>
                      </div>
                    ) : (
                      <div className="loading-status">
                        <Loader2 size={14} className="spin" />
                        <span>
                          {state.imageDataUrl
                            ? "AI 正在结合截图分析回答..."
                            : "AI 正在结合文本分析回答..."}
                        </span>
                      </div>
                    ))}

                  {state.error && <div className="error-box">{state.error}</div>}
                  <div ref={threadEndRef} />
                </div>
              </>
            )}
          </div>
        </div>

        {(currentMode === "translate" && (state.status === "loading" || doneTranslate)) && (
          <div className="result-footer" ref={footerRef}>
            <div className="result-footer-left">
              <select
                className="result-footer-select"
                aria-label="翻译引擎"
                value={activeEngine}
                onChange={(e) => {
                  const nextEngine = e.target.value as TranslateEngine;
                  setActiveEngine(nextEngine);
                  switchMode("translate", nextEngine, activeTargetLang);
                }}
              >
                {ENGINE_OPTIONS.map((eng) => (
                  <option key={eng.id} value={eng.id}>
                    {eng.id === "ai" ? "AI" : eng.id === "google" ? "Google" : "DeepL"}
                  </option>
                ))}
              </select>
              <span className="result-footer-sep">→</span>
              <select
                className="result-footer-select"
                aria-label="目标语言"
                value={activeTargetLang}
                onChange={(e) => {
                  const nextLang = e.target.value;
                  setActiveTargetLang(nextLang);
                  switchMode("translate", activeEngine, nextLang);
                }}
              >
                {TARGET_LANGUAGES.map((lang) => (
                  <option key={lang} value={lang}>
                    {lang}
                  </option>
                ))}
              </select>
              {doneTranslate && doneTranslate.originalText && (
                <button
                  type="button"
                  className={`mini-chip result-orig-toggle ${showOriginal ? "active" : ""}`}
                  onClick={() => setShowOriginal((v) => !v)}
                  title={showOriginal ? "收起原文" : "展开原文与编辑"}
                >
                  {showOriginal ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
                  原文
                </button>
              )}
            </div>

            {doneTranslate && (
              <div
                className="result-footer-right"
                title={`${doneTranslate.sourceLang} → ${doneTranslate.targetLang} · ${doneTranslate.model} · ${doneTranslate.elapsedMs}ms`}
              >
                <button
                  type="button"
                  className="icon-btn"
                  onClick={() =>
                    toggleSpeak(doneTranslate.translatedText, "trans", doneTranslate.targetLang)
                  }
                  title="朗读译文"
                >
                  {speakingKey === "trans" ? <VolumeX size={13} /> : <Volume2 size={13} />}
                </button>
                <button
                  type="button"
                  className="icon-btn"
                  onClick={() =>
                    invoke("retry_translation", {
                      engineOverride: activeEngine,
                      targetLangOverride: activeTargetLang,
                    })
                  }
                  title={`重新翻译 (${doneTranslate.elapsedMs}ms)`}
                >
                  <RotateCw size={12} />
                </button>
                {displayOriginal && (
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() =>
                      copyText(
                        `${displayOriginal}\n---\n${doneTranslate.translatedText}`,
                        "bilingual"
                      )
                    }
                    title="同时复制原文与译文"
                  >
                    {copiedKey === "bilingual" ? <Check size={11} /> : "双语"}
                  </button>
                )}
                <button
                  type="button"
                  className="btn btn-sm btn-primary"
                  onClick={() => copyText(doneTranslate.translatedText, "trans")}
                >
                  {copiedKey === "trans" ? (
                    <>
                      <Check size={11} /> 已复制
                    </>
                  ) : (
                    <>
                      <Copy size={11} /> 复制
                    </>
                  )}
                </button>
              </div>
            )}
          </div>
        )}

        {doneOcr && (
          <div className="result-footer" ref={footerRef}>
            <div className="result-footer-left">
              <span className="result-footer-meta">
                {doneOcr.sourceLang} · {displayOcr.length} 字 · {doneOcr.elapsedMs}ms
              </span>
              {hasMergeableOcrLines && (
                <button
                  type="button"
                  className={`mini-chip ${mergeLines ? "active" : ""}`}
                  onClick={() => setMergeLines((v) => !v)}
                  title="去除识别结果中句子中间的多余单行换行（保留段落空行）"
                >
                  <WrapText size={11} style={{ verticalAlign: "-1px", marginRight: 2 }} />
                  合并
                </button>
              )}
            </div>
            <div className="result-footer-right">
              <button
                type="button"
                className="icon-btn"
                onClick={() => toggleSpeak(displayOcr, "ocr", doneOcr.sourceLang)}
                title="朗读文字"
              >
                {speakingKey === "ocr" ? <VolumeX size={13} /> : <Volume2 size={13} />}
              </button>
              <button
                type="button"
                className="btn btn-sm"
                onClick={() =>
                  mergeLines
                    ? invoke("retranslate_custom_text", {
                        text: displayOcr,
                        engineOverride: activeEngine,
                        targetLangOverride: activeTargetLang,
                      }).catch(console.error)
                    : switchMode("translate", activeEngine, activeTargetLang)
                }
              >
                <Languages size={11} /> 翻译
              </button>
              <button
                type="button"
                className="btn btn-sm btn-primary"
                onClick={() => copyText(displayOcr, "ocr")}
              >
                {copiedKey === "ocr" ? (
                  <>
                    <Check size={11} /> 已复制
                  </>
                ) : (
                  <>
                    <Copy size={11} /> 复制
                  </>
                )}
              </button>
            </div>
          </div>
        )}

        {state.status === "ask" && (
          <>
            <AskQuickActionsBar
              loading={state.loading}
              onTrigger={(prompt) => void handleAskSend(prompt)}
            />
            <div className="ask-input-dock">
              <textarea
                className="ask-textarea"
                rows={1}
                value={askInput}
                onChange={(e) => setAskInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    handleAskSend();
                  }
                }}
                placeholder={
                  state.imageDataUrl
                    ? "问 AI：截图里有什么？这段话什么意思？(Enter 发送)"
                    : "问 AI：解释这段话、总结要点或继续追问... (Enter 发送)"
                }
                disabled={state.loading}
              />
              <button
                type="button"
                className="btn btn-primary btn-sm"
                style={{ height: 33, padding: "0 12px" }}
                onClick={() => handleAskSend()}
                disabled={state.loading || !askInput.trim()}
              >
                <Send size={13} />
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
