import { Keyboard, X } from "lucide-react";
import { ALT_LABEL } from "../utils/platform";

export interface SelectionShortcutFieldProps {
  value: string;
  savedValue?: string;
  recording: boolean;
  error?: string | null;
  onChange: (next: string) => void;
  onToggleRecord: () => void;
}

const SELECTION_SHORTCUT_PRESETS = [
  { value: "", label: "不启用" },
  { value: "Space", label: "空格 (Space)" },
  { value: "Tab", label: "Tab" },
  { value: "Alt+E", label: `${ALT_LABEL}+E` },
] as const;

export function SelectionShortcutField({
  value,
  savedValue,
  recording,
  error,
  onChange,
  onToggleRecord,
}: SelectionShortcutFieldProps) {
  const normalized = value.trim();
  const isSmartSingleKey =
    normalized.toLowerCase() === "space" || normalized.toLowerCase() === "tab";
  const unsaved = normalized !== (savedValue ?? "");

  return (
    <>
      <div className="field" style={{ marginTop: 12 }}>
        <label htmlFor="selection-shortcut-input">
          全局划词翻译快捷键（不污染剪贴板，选中任意软件文字后原地弹译文）
          {unsaved && <span className="unsaved-hint">未保存</span>}
        </label>
        <div className="input-with-btn">
          <input
            id="selection-shortcut-input"
            className="input"
            value={
              recording
                ? "请按下快捷键，支持单键 Space / Tab 或组合键（Esc 取消）..."
                : value
            }
            readOnly={recording}
            onChange={(e) => onChange(e.target.value)}
            placeholder={`留空则不启用，如 Space、Tab 或 ${ALT_LABEL}+E`}
          />
          <button
            type="button"
            className={`btn ${recording ? "btn-primary" : ""}`}
            onClick={onToggleRecord}
            title="录制全局划词翻译快捷键（支持单键 Space / Tab 或组合键）"
          >
            <Keyboard size={14} />
            {recording ? "取消" : "录制"}
          </button>
          {value && (
            <button
              type="button"
              className="btn"
              onClick={() => onChange("")}
              title="清除并禁用此快捷键"
              aria-label="清除全局划词翻译快捷键"
            >
              <X size={14} />
            </button>
          )}
        </div>
        <div className="chip-row">
          {SELECTION_SHORTCUT_PRESETS.map((sc) => (
            <button
              key={sc.value || "none"}
              type="button"
              className={`chip ${normalized.toLowerCase() === sc.value.toLowerCase() ? "active" : ""}`}
              onClick={() => onChange(sc.value)}
            >
              {sc.label}
            </button>
          ))}
        </div>
        {isSmartSingleKey && (
          <small className="selection-smart-hint">
            智能单键：鼠标划选或双击选中文字后轻按 {normalized === "Space" || normalized.toLowerCase() === "space" ? "Space（空格）" : "Tab"} 即可触发翻译；平时正常打字、切焦点与全屏游戏/视频均自动避让不受影响。
          </small>
        )}
      </div>

      {error && (
        <div className="error-box" style={{ marginTop: 8 }}>
          {error}
        </div>
      )}
    </>
  );
}
