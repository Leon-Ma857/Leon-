import { useState, type KeyboardEvent } from "react";
import { Check, ChevronDown, ChevronUp, Pencil, Plus, RotateCcw, Trash2, X } from "lucide-react";
import { AskQuickAction, DEFAULT_ASK_QUICK_ACTIONS } from "../types";
import { MOD_LABEL } from "../utils/platform";
import { Popconfirm } from "./Popconfirm";
import "../styles/ask-actions.css";

const MAX_ASK_ACTIONS = 20;
const MAX_LABEL_LENGTH = 24;
const MAX_PROMPT_LENGTH = 1200;

export interface AskActionsSettingsProps {
  actions: AskQuickAction[];
  onChange: (next: AskQuickAction[]) => void;
}

function charLength(value: string): number {
  return Array.from(value).length;
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

export function AskActionsSettings({ actions, onChange }: AskActionsSettingsProps) {
  const safeActions = Array.isArray(actions) ? actions : [];
  const isFull = safeActions.length >= MAX_ASK_ACTIONS;

  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [editLabel, setEditLabel] = useState("");
  const [editPrompt, setEditPrompt] = useState("");
  const [editError, setEditError] = useState<string | null>(null);

  const [newLabel, setNewLabel] = useState("");
  const [newPrompt, setNewPrompt] = useState("");
  const [addError, setAddError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const startEdit = (index: number) => {
    const item = safeActions[index];
    if (!item) return;
    setEditingIndex(index);
    setEditLabel(item.label);
    setEditPrompt(item.prompt);
    setEditError(null);
    setNotice(null);
  };

  const cancelEdit = () => {
    setEditingIndex(null);
    setEditLabel("");
    setEditPrompt("");
    setEditError(null);
  };

  const saveEdit = (index: number) => {
    const item = safeActions[index];
    if (!item) return;
    const label = truncateChars(editLabel.trim(), MAX_LABEL_LENGTH);
    const prompt = truncateChars(editPrompt.trim(), MAX_PROMPT_LENGTH);
    if (!label && !prompt) {
      setEditError("请输入快捷动作简称和完整提示词");
      return;
    }
    if (!label) {
      setEditError("请输入快捷动作简称（最多 24 字）");
      return;
    }
    if (!prompt) {
      setEditError("请输入完整提示词");
      return;
    }

    const next = safeActions.map((cur, idx) =>
      idx === index ? { ...cur, label, prompt } : cur
    );
    onChange(next);
    cancelEdit();
    setNotice("已更新，保存设置后生效");
  };

  const moveAction = (index: number, delta: -1 | 1) => {
    const target = index + delta;
    if (target < 0 || target >= safeActions.length) return;
    const next = [...safeActions];
    const [moved] = next.splice(index, 1);
    next.splice(target, 0, moved);
    if (editingIndex === index) {
      setEditingIndex(target);
    } else if (editingIndex === target) {
      setEditingIndex(index);
    }
    setNotice(null);
    onChange(next);
  };

  const deleteAction = (index: number) => {
    if (editingIndex === index) {
      cancelEdit();
    } else if (editingIndex !== null && editingIndex > index) {
      setEditingIndex(editingIndex - 1);
    }
    setAddError(null);
    setNotice(null);
    onChange(safeActions.filter((_, idx) => idx !== index));
  };

  const handleAdd = () => {
    setNotice(null);
    if (isFull) {
      setAddError(`已达到最多 ${MAX_ASK_ACTIONS} 个快捷动作上限，请先删除不需要的预设后再添加`);
      return;
    }
    const label = truncateChars(newLabel.trim(), MAX_LABEL_LENGTH);
    const prompt = truncateChars(newPrompt.trim(), MAX_PROMPT_LENGTH);
    if (!label && !prompt) {
      setAddError("请输入快捷动作简称和完整提示词");
      return;
    }
    if (!label) {
      setAddError("请输入快捷动作简称（最多 24 字）");
      return;
    }
    if (!prompt) {
      setAddError("请输入完整提示词");
      return;
    }

    const next: AskQuickAction[] = [
      ...safeActions,
      {
        id: createActionId(safeActions),
        label,
        prompt,
      },
    ];
    onChange(next);
    setNewLabel("");
    setNewPrompt("");
    setAddError(null);
  };

  const handleResetDefaults = () => {
    cancelEdit();
    setAddError(null);
    onChange(DEFAULT_ASK_QUICK_ACTIONS.map((item) => ({ ...item })));
    setNotice("已恢复默认，保存后生效");
  };

  const handleEditInputKeyDown = (index: number, e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && !e.nativeEvent.isComposing) {
      e.preventDefault();
      saveEdit(index);
    } else if (e.key === "Escape") {
      e.preventDefault();
      cancelEdit();
    }
  };

  const handleEditTextareaKeyDown = (index: number, e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.nativeEvent.isComposing) {
      e.preventDefault();
      saveEdit(index);
    } else if (e.key === "Escape") {
      e.preventDefault();
      cancelEdit();
    }
  };

  const handleAddInputKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && !e.nativeEvent.isComposing) {
      e.preventDefault();
      handleAdd();
    }
  };

  const handleAddTextareaKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.nativeEvent.isComposing) {
      e.preventDefault();
      handleAdd();
    }
  };

  return (
    <section className="card ask-actions-settings">
      <div className="card-title ask-settings-header">
        <div className="ask-settings-heading">
          <span>快捷指令</span>
          <span className="card-subtitle">显示在「问 AI」输入框上方，点击一键提问</span>
        </div>
        <div className="ask-settings-header-actions">
          <span className="ask-settings-count">
            {safeActions.length} / {MAX_ASK_ACTIONS}
          </span>
          <Popconfirm
            message="恢复为默认快捷指令？"
            confirmLabel="恢复"
            danger={false}
            onConfirm={handleResetDefaults}
          >
            {(triggerProps) => (
              <button
                type="button"
                className="btn btn-sm"
                title="恢复默认快捷指令"
                {...triggerProps}
              >
                <RotateCcw size={12} /> 恢复默认
              </button>
            )}
          </Popconfirm>
        </div>
      </div>

      <div className="ask-settings-list">
        {safeActions.length === 0 ? (
          <div className="ask-settings-empty">
            暂无快捷指令，可在下方添加或恢复默认。
          </div>
        ) : (
          safeActions.map((action, index) => {
            const isEditing = editingIndex === index;
            return (
              <div
                key={`${action.id}-${index}`}
                className={`ask-settings-row${isEditing ? " editing" : ""}`}
              >
                {isEditing ? (
                  <div className="ask-settings-edit-form">
                    <div className="ask-settings-edit-head">
                      <label>简称（最多 {MAX_LABEL_LENGTH} 字）</label>
                      <span className="ask-settings-char-count">
                        {charLength(editLabel)}/{MAX_LABEL_LENGTH}
                      </span>
                    </div>
                    <input
                      type="text"
                      className="input"
                      maxLength={MAX_LABEL_LENGTH}
                      value={editLabel}
                      onChange={(e) => {
                        setEditLabel(e.target.value);
                        if (editError) setEditError(null);
                      }}
                      onKeyDown={(e) => handleEditInputKeyDown(index, e)}
                      placeholder="例如：转 Markdown 表格"
                      autoFocus
                    />

                    <div className="ask-settings-edit-head">
                      <label>完整提示词</label>
                      <span className="ask-settings-char-count">{MOD_LABEL}+Enter 保存</span>
                    </div>
                    <textarea
                      className="textarea"
                      rows={2}
                      maxLength={MAX_PROMPT_LENGTH}
                      value={editPrompt}
                      onChange={(e) => {
                        setEditPrompt(e.target.value);
                        if (editError) setEditError(null);
                      }}
                      onKeyDown={(e) => handleEditTextareaKeyDown(index, e)}
                      placeholder="输入点击该胶囊后发送给 AI 的完整指令..."
                    />

                    <div className="ask-settings-edit-footer">
                      <div className="ask-settings-edit-msg">
                        {editError && <span className="ask-settings-hint err">{editError}</span>}
                      </div>
                      <div className="ask-settings-edit-btns">
                        <button
                          type="button"
                          className="btn btn-sm"
                          onClick={cancelEdit}
                        >
                          <X size={12} /> 取消
                        </button>
                        <button
                          type="button"
                          className="btn btn-sm btn-primary"
                          onClick={() => saveEdit(index)}
                        >
                          <Check size={12} /> 保存修改
                        </button>
                      </div>
                    </div>
                  </div>
                ) : (
                  <>
                    <div className="ask-settings-row-top">
                      <div className="ask-settings-row-title">
                        <span className="ask-settings-index">{index + 1}</span>
                        <span className="ask-settings-pill-preview">{action.label}</span>
                      </div>
                      <div className="ask-settings-row-actions">
                        <button
                          type="button"
                          className="ask-settings-icon-btn"
                          onClick={() => moveAction(index, -1)}
                          disabled={index === 0}
                          title="上移"
                          aria-label={`上移 ${action.label}`}
                        >
                          <ChevronUp size={14} />
                        </button>
                        <button
                          type="button"
                          className="ask-settings-icon-btn"
                          onClick={() => moveAction(index, 1)}
                          disabled={index === safeActions.length - 1}
                          title="下移"
                          aria-label={`下移 ${action.label}`}
                        >
                          <ChevronDown size={14} />
                        </button>
                        <button
                          type="button"
                          className="ask-settings-icon-btn"
                          onClick={() => startEdit(index)}
                          title="编辑"
                          aria-label={`编辑 ${action.label}`}
                        >
                          <Pencil size={13} />
                        </button>
                        <button
                          type="button"
                          className="ask-settings-icon-btn danger"
                          onClick={() => deleteAction(index)}
                          title="删除"
                          aria-label={`删除 ${action.label}`}
                        >
                          <Trash2 size={13} />
                        </button>
                      </div>
                    </div>
                    <div className="ask-settings-prompt-preview">{action.prompt}</div>
                  </>
                )}
              </div>
            );
          })
        )}
      </div>

      <div className="ask-settings-add-box">
        <div className="ask-settings-add-head">
          <span>新增指令</span>
          <span className="ask-settings-char-count">
            {charLength(newLabel)}/{MAX_LABEL_LENGTH}
          </span>
        </div>

        <div className="ask-settings-add-fields">
          <input
            type="text"
            className="input"
            maxLength={MAX_LABEL_LENGTH}
            value={newLabel}
            onChange={(e) => {
              setNewLabel(e.target.value);
              if (addError) setAddError(null);
            }}
            onKeyDown={handleAddInputKeyDown}
            placeholder="名称，如：转 CSV / 提取待办"
          />
          <textarea
            className="textarea"
            rows={2}
            maxLength={MAX_PROMPT_LENGTH}
            value={newPrompt}
            onChange={(e) => {
              setNewPrompt(e.target.value);
              if (addError) setAddError(null);
            }}
            onKeyDown={handleAddTextareaKeyDown}
            placeholder="发送给 AI 的提示词…"
          />
        </div>

        <div className="ask-settings-add-footer">
          <div className="ask-settings-hint-slot" role="status">
            {addError ? (
              <span className="ask-settings-hint err">{addError}</span>
            ) : isFull ? (
              <span className="ask-settings-hint warn">
                已达上限（{MAX_ASK_ACTIONS}/{MAX_ASK_ACTIONS}）
              </span>
            ) : notice ? (
              <span className="ask-settings-hint ok">{notice}</span>
            ) : (
              <span className="ask-settings-hint">
                保存后同步到所有窗口
              </span>
            )}
          </div>
          <button
            type="button"
            className="btn btn-sm btn-primary"
            onClick={handleAdd}
            disabled={isFull}
          >
            <Plus size={13} /> 添加
          </button>
        </div>
      </div>
    </section>
  );
}

export default AskActionsSettings;
