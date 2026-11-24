import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type WheelEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Plus, RotateCcw, X } from "lucide-react";
import { AskQuickAction, DEFAULT_ASK_QUICK_ACTIONS } from "../types";
import "../styles/ask-actions.css";

const MAX_ASK_ACTIONS = 20;
const MAX_LABEL_LENGTH = 24;
const MAX_PROMPT_LENGTH = 1200;

export interface AskQuickActionsBarProps {
  loading: boolean;
  onTrigger: (prompt: string) => void;
  compact?: boolean;
}

function truncateChars(value: string, max: number): string {
  return Array.from(value).slice(0, max).join("");
}

function createActionId(existing: AskQuickAction[]): string {
  const used = new Set(existing.map((item) => item.id));
  const stamp = Date.now().toString(36);
  let seq = 1;
  while (used.has(`custom-${stamp}-${seq}`)) {
    seq += 1;
  }
  return `custom-${stamp}-${seq}`;
}

export function AskQuickActionsBar({
  loading,
  onTrigger,
  compact = false,
}: AskQuickActionsBarProps) {
  const [actions, setActions] = useState<AskQuickAction[]>(DEFAULT_ASK_QUICK_ACTIONS);
  const [editorOpen, setEditorOpen] = useState(false);
  const [newLabel, setNewLabel] = useState("");
  const [newPrompt, setNewPrompt] = useState("");
  const [saving, setSaving] = useState(false);
  const [feedback, setFeedback] = useState<{ kind: "err" | "ok"; text: string } | null>(null);

  const barRef = useRef<HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const labelInputRef = useRef<HTMLInputElement | null>(null);
  const promptInputRef = useRef<HTMLInputElement | null>(null);
  const feedbackTimerRef = useRef<number | null>(null);

  const clearFeedbackTimer = useCallback(() => {
    if (feedbackTimerRef.current !== null) {
      window.clearTimeout(feedbackTimerRef.current);
      feedbackTimerRef.current = null;
    }
  }, []);

  const showFeedback = useCallback(
    (kind: "err" | "ok", text: string) => {
      clearFeedbackTimer();
      setFeedback({ kind, text });
      if (kind === "ok") {
        feedbackTimerRef.current = window.setTimeout(() => {
          setFeedback((cur) => (cur?.kind === "ok" && cur.text === text ? null : cur));
          feedbackTimerRef.current = null;
        }, 2000);
      }
    },
    [clearFeedbackTimer]
  );

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    invoke<AskQuickAction[]>("get_ask_quick_actions")
      .then((list) => {
        if (!disposed && Array.isArray(list)) {
          setActions(list);
        }
      })
      .catch(() => {
        if (!disposed) {
          setActions(DEFAULT_ASK_QUICK_ACTIONS);
        }
      });

    listen<AskQuickAction[]>("ask-actions:updated", (event) => {
      if (!disposed && Array.isArray(event.payload)) {
        setActions(event.payload);
      }
    })
      .then((fn) => {
        if (disposed) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch(console.error);

    return () => {
      disposed = true;
      clearFeedbackTimer();
      if (unlisten) unlisten();
    };
  }, [clearFeedbackTimer]);

  useEffect(() => {
    if (!editorOpen) return;

    const raf = window.requestAnimationFrame(() => {
      labelInputRef.current?.focus();
    });

    const onDocMouseDown = (e: MouseEvent) => {
      if (barRef.current && !barRef.current.contains(e.target as Node)) {
        setEditorOpen(false);
      }
    };

    const onWindowKeyDown = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setEditorOpen(false);
      }
    };

    document.addEventListener("mousedown", onDocMouseDown);
    window.addEventListener("keydown", onWindowKeyDown, true);
    return () => {
      window.cancelAnimationFrame(raf);
      document.removeEventListener("mousedown", onDocMouseDown);
      window.removeEventListener("keydown", onWindowKeyDown, true);
    };
  }, [editorOpen]);

  const handleWheel = (e: WheelEvent<HTMLDivElement>) => {
    const el = scrollRef.current;
    if (!el || el.scrollWidth <= el.clientWidth) return;
    const unit = e.deltaMode === 1 ? 24 : 1;
    const dx = e.deltaX * unit;
    const dy = e.deltaY * unit;
    const delta = Math.abs(dy) >= Math.abs(dx) ? dy : dx;
    if (delta !== 0) {
      el.scrollLeft += delta;
    }
  };

  const persistActions = async (next: AskQuickAction[], okText?: string): Promise<boolean> => {
    setSaving(true);
    clearFeedbackTimer();
    setFeedback(null);
    try {
      const saved = await invoke<AskQuickAction[]>("save_ask_quick_actions", { actions: next });
      setActions(Array.isArray(saved) ? saved : next);
      if (okText) {
        showFeedback("ok", okText);
      }
      return true;
    } catch (err) {
      showFeedback("err", `保存失败：${String(err)}`);
      return false;
    } finally {
      setSaving(false);
    }
  };

  const handleAddAction = async () => {
    if (saving) return;
    if (actions.length >= MAX_ASK_ACTIONS) {
      showFeedback("err", `最多 ${MAX_ASK_ACTIONS} 个快捷指令，请先删除旧项`);
      return;
    }
    const label = truncateChars(newLabel.trim(), MAX_LABEL_LENGTH);
    const prompt = truncateChars(newPrompt.trim(), MAX_PROMPT_LENGTH);
    if (!label) {
      showFeedback("err", "请输入名称");
      labelInputRef.current?.focus();
      return;
    }
    if (!prompt) {
      showFeedback("err", "请输入提示词");
      promptInputRef.current?.focus();
      return;
    }

    const next = [
      ...actions,
      {
        id: createActionId(actions),
        label,
        prompt,
      },
    ];
    const ok = await persistActions(next, "已添加");
    if (ok) {
      setNewLabel("");
      setNewPrompt("");
      window.requestAnimationFrame(() => {
        if (scrollRef.current) {
          scrollRef.current.scrollLeft = scrollRef.current.scrollWidth;
        }
        labelInputRef.current?.focus();
      });
    }
  };

  const handleDeleteAction = async (actionId: string) => {
    if (saving) return;
    const next = actions.filter((item) => item.id !== actionId);
    await persistActions(next);
  };

  const handleResetDefaults = async () => {
    if (saving) return;
    const defaults = DEFAULT_ASK_QUICK_ACTIONS.map((item) => ({ ...item }));
    await persistActions(defaults, "已恢复默认");
  };

  const handleInputKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    e.stopPropagation();
    if (e.key === "Enter" && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void handleAddAction();
    } else if (e.key === "Escape") {
      e.preventDefault();
      setEditorOpen(false);
    }
  };

  const isFull = actions.length >= MAX_ASK_ACTIONS;

  return (
    <div ref={barRef} className={`ask-actions-bar${compact ? " compact" : ""}`}>
      {editorOpen && (
        <div
          className="ask-actions-editor"
          role="dialog"
          aria-label="自定义快捷指令"
        >
          <div className="ask-editor-head">
            <span className="ask-editor-title">
              快捷指令 ({actions.length}/{MAX_ASK_ACTIONS})
            </span>
            <button
              type="button"
              className="ask-editor-btn"
              onClick={() => void handleResetDefaults()}
              disabled={saving}
              title="恢复默认"
            >
              <RotateCcw size={11} />
              恢复默认
            </button>
          </div>

          <div className="ask-editor-form">
            <div className="ask-editor-form-row">
              <input
                ref={labelInputRef}
                type="text"
                className="ask-editor-input"
                maxLength={MAX_LABEL_LENGTH}
                value={newLabel}
                disabled={saving}
                onChange={(e) => {
                  setNewLabel(e.target.value);
                  if (feedback?.kind === "err") setFeedback(null);
                }}
                onKeyDown={handleInputKeyDown}
                placeholder="名称，如：转 CSV"
              />
              <button
                type="button"
                className="ask-editor-btn ask-editor-btn-primary"
                onClick={() => void handleAddAction()}
                disabled={saving || isFull}
                title={isFull ? `最多 ${MAX_ASK_ACTIONS} 个` : "保存"}
              >
                {saving ? "保存中…" : "保存"}
              </button>
            </div>
            <input
              ref={promptInputRef}
              type="text"
              className="ask-editor-input"
              maxLength={MAX_PROMPT_LENGTH}
              value={newPrompt}
              disabled={saving}
              onChange={(e) => {
                setNewPrompt(e.target.value);
                if (feedback?.kind === "err") setFeedback(null);
              }}
              onKeyDown={handleInputKeyDown}
              placeholder="提示词…"
            />
          </div>

          {feedback && (
            <div className={`ask-editor-feedback ${feedback.kind}`} role="status">
              {feedback.text}
            </div>
          )}

          {!feedback && isFull && (
            <div className="ask-editor-feedback err" role="status">
              已达上限 ({MAX_ASK_ACTIONS})，删除旧项后可继续添加
            </div>
          )}

          <div className="ask-editor-list">
            {actions.length === 0 ? (
              <div className="ask-editor-empty">暂无快捷指令</div>
            ) : (
              actions.map((action) => (
                <div key={action.id} className="ask-editor-item">
                  <div className="ask-editor-item-main" title={action.prompt}>
                    <span className="ask-editor-item-label">{action.label}</span>
                    <span className="ask-editor-item-prompt">{action.prompt}</span>
                  </div>
                  <button
                    type="button"
                    className="ask-editor-del-btn"
                    onClick={() => void handleDeleteAction(action.id)}
                    disabled={saving}
                    title={`删除 ${action.label}`}
                    aria-label={`删除 ${action.label}`}
                  >
                    ×
                  </button>
                </div>
              ))
            )}
          </div>
        </div>
      )}

      <div className="ask-actions-row">
        <div
          ref={scrollRef}
          className="ask-actions-scroll"
          onWheel={handleWheel}
        >
          {actions.length === 0 ? (
            <button
              type="button"
              className="ask-actions-empty-hint"
              onClick={() => setEditorOpen(true)}
            >
              点击 + 添加快捷指令
            </button>
          ) : (
            actions.map((action) => (
              <button
                key={action.id}
                type="button"
                className="ask-action-pill"
                title={action.prompt}
                disabled={loading}
                onClick={() => {
                  if (editorOpen) setEditorOpen(false);
                  onTrigger(action.prompt);
                }}
              >
                {action.label}
              </button>
            ))
          )}
        </div>
        <button
          type="button"
          className={`ask-action-add-btn${editorOpen ? " active" : ""}`}
          title="自定义快捷指令"
          aria-label="自定义快捷指令"
          aria-expanded={editorOpen}
          onClick={() => {
            clearFeedbackTimer();
            setFeedback(null);
            setEditorOpen((v) => !v);
          }}
        >
          {editorOpen ? <X size={compact ? 12 : 13} /> : <Plus size={compact ? 12 : 13} />}
        </button>
      </div>
    </div>
  );
}

export default AskQuickActionsBar;
