import { Keyboard, X } from "lucide-react";
import { formatShortcutLabel } from "../utils/platform";

export interface ShortcutPresetOption {
  value: string;
  label: string;
}

export interface ShortcutRowProps {
  label: string;
  desc?: string;
  value: string;
  savedValue?: string;
  recording: boolean;
  optional?: boolean;
  presets?: readonly ShortcutPresetOption[];
  recordingPrompt?: string;
  error?: string | null;
  hint?: string | null;
  onChange: (next: string) => void;
  onToggleRecord: () => void;
}

function splitKeycaps(accelerator: string): string[] {
  const formatted = formatShortcutLabel(accelerator.trim());
  if (!formatted) return [];
  return formatted.split("+").filter(Boolean);
}

export function ShortcutRow({
  label,
  desc,
  value,
  savedValue,
  recording,
  optional = false,
  presets,
  recordingPrompt = "按下组合键… (Esc 取消)",
  error,
  hint,
  onChange,
  onToggleRecord,
}: ShortcutRowProps) {
  const trimmed = value.trim();
  const savedTrimmed = (savedValue ?? "").trim();
  const unsaved = savedValue !== undefined && trimmed !== savedTrimmed;
  const keycaps = splitKeycaps(trimmed);

  return (
    <div className="setting-row shortcut-setting-row">
      <div className="shortcut-row-top">
        <div className="shortcut-row-info">
          <div className="s-label">
            <span>{label}</span>
            {unsaved && (
              <span className="s-unsaved-pill" title={savedTrimmed ? `当前生效：${formatShortcutLabel(savedTrimmed)}` : "尚未保存"}>
                未保存
              </span>
            )}
          </div>
          {desc && <div className="s-desc">{desc}</div>}
        </div>

        <div className="shortcut-recorder-group">
          <button
            type="button"
            className={`shortcut-recorder-btn${recording ? " is-recording" : ""}${!trimmed ? " is-empty" : ""}`}
            onClick={onToggleRecord}
            title={recording ? "按 Esc 或再次点击取消录制" : "点击后直接按下快捷键录制"}
          >
            {recording ? (
              <span className="shortcut-recording-text">
                <span className="shortcut-rec-dot" aria-hidden="true" />
                {recordingPrompt}
              </span>
            ) : keycaps.length > 0 ? (
              <span className="shortcut-keycaps">
                {keycaps.map((cap, idx) => (
                  <kbd key={`${cap}-${idx}`} className="kbd-cap">
                    {cap}
                  </kbd>
                ))}
              </span>
            ) : (
              <span className="shortcut-empty-label">
                <Keyboard size={12} /> 点击录制
              </span>
            )}
          </button>

          {optional && trimmed && !recording && (
            <button
              type="button"
              className="icon-btn shortcut-clear-btn"
              onClick={() => onChange("")}
              title={`清除${label}`}
              aria-label={`清除${label}`}
            >
              <X size={13} />
            </button>
          )}
        </div>
      </div>

      {presets && presets.length > 0 && (
        <div className="shortcut-preset-row">
          <span className="shortcut-preset-caption">预设</span>
          {presets.map((p) => {
            const active = trimmed.toLowerCase() === p.value.toLowerCase();
            return (
              <button
                key={p.value || "off"}
                type="button"
                className={`kbd-preset-btn${active ? " active" : ""}`}
                onClick={() => onChange(p.value)}
              >
                {p.label}
              </button>
            );
          })}
        </div>
      )}

      {hint && <div className="shortcut-row-hint">{hint}</div>}
      {error && <div className="error-box shortcut-row-error">{error}</div>}
    </div>
  );
}
