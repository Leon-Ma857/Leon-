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
  { value: "", label: "关闭" },
  { value: "Space", label: "Space" },
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
          划词翻译
          {unsaved && <span className="unsaved-hint">未保存</span>}
        </label>
        <div className="input-with-btn">
          <input
            id="selection-shortcut-input"
            className="input"
            value={
              recording
                ? "按下快捷键，支持 Space / Tab（Esc 取消）"
                : value
            }
            readOnly={recording}
            onChange={(e) => onChange(e.target.value)}
            placeholder={`留空禁用，如 Space、Tab 或 ${ALT_LABEL}+E`}
          />
          <button
            type="button"
            className={`btn ${recording ? "btn-primary" : ""}`}
            onClick={onToggleRecord}
            title="录制划词快捷键"
          >
            <Keyboard size={14} />
            {recording ? "取消" : "录制"}
          </button>
          {value && (
            <button
              type="button"
              className="btn"
              onClick={() => onChange("")}
              title="清除快捷键"
              aria-label="清除划词翻译快捷键"
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
            选中文字后轻按 {normalized.toLowerCase() === "space" ? "Space" : "Tab"} 即可翻译，正常输入与全屏游戏自动避让。
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
