import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Download,
  FolderUp,
  Play,
  RefreshCw,
  Trash2,
  X,
} from "lucide-react";
import { formatBytes } from "../appUpdate";
import {
  LocalOcrEngineMode,
  LocalOcrLang,
  LocalOcrProgress,
  LocalOcrStatus,
} from "../types";
import { listenToWindow } from "../utils/windowEvents";
import { IS_MAC } from "../utils/platform";
import { Popconfirm } from "./Popconfirm";
import "../styles/local-ocr.css";

export interface LocalOcrSettingsProps {
  engineMode: LocalOcrEngineMode;
  ocrLang: LocalOcrLang;
  downloadUrl: string;
  proxyUrl: string;
  onChangeEngineMode: (mode: LocalOcrEngineMode) => void;
  onChangeOcrLang: (lang: LocalOcrLang) => void;
  onChangeDownloadUrl: (url: string) => void;
  onNotice: (kind: "ok" | "err", text: string) => void;
}

const ENGINE_MODES: {
  id: LocalOcrEngineMode;
  label: string;
  desc: string;
}[] = [
  {
    id: "auto",
    label: "自动",
    desc: "优先 RapidOCR，未安装时回退系统 OCR。",
  },
  {
    id: "rapidocr",
    label: "RapidOCR",
    desc: "仅使用本地 RapidOCR 离线识字，不上传云端。",
  },
  {
    id: "windows",
    label: "系统 OCR",
    desc: "仅使用 Windows 内置识字。",
  },
];

const OCR_LANG_OPTIONS: { id: LocalOcrLang; label: string }[] = [
  { id: "ch", label: "中英混排" },
  { id: "cht", label: "繁体中文" },
  { id: "en", label: "English" },
  { id: "jp", label: "日本語" },
  { id: "kr", label: "한국어" },
  { id: "ru", label: "Русский" },
];

function stageLabel(progress: LocalOcrProgress | null): string {
  if (!progress) return "正在准备离线引擎…";
  switch (progress.stage) {
    case "downloading":
      return progress.message || "正在下载 RapidOCR 引擎包…";
    case "verifying":
      return progress.message || "正在校验 SHA-256 完整性…";
    case "extracting":
      return progress.message || "正在解压并部署模型…";
    case "ready":
      return progress.message || "离线 RapidOCR 引擎已就绪";
    case "cancelled":
      return progress.message || "已取消下载";
    case "error":
      return progress.message || "处理失败";
    default:
      return progress.message || "正在处理…";
  }
}

export function LocalOcrSettings({
  engineMode,
  ocrLang,
  downloadUrl,
  proxyUrl,
  onChangeEngineMode,
  onChangeOcrLang,
  onChangeDownloadUrl,
  onNotice,
}: LocalOcrSettingsProps) {
  const [status, setStatus] = useState<LocalOcrStatus | null>(null);
  const [loadingStatus, setLoadingStatus] = useState(true);
  const [progress, setProgress] = useState<LocalOcrProgress | null>(null);
  const [actionBusy, setActionBusy] = useState<
    null | "download" | "import" | "uninstall" | "test"
  >(null);
  const [cancelling, setCancelling] = useState(false);
  const [testOutput, setTestOutput] = useState<string | null>(null);
  const [showMirror, setShowMirror] = useState(() => Boolean(downloadUrl.trim()));
  const sawReadyRef = useRef(false);

  const refreshStatus = useCallback(async () => {
    try {
      const next = await invoke<LocalOcrStatus>("get_local_ocr_status");
      setStatus(next);
    } catch (err) {
      console.error(err);
    } finally {
      setLoadingStatus(false);
    }
  }, []);

  useEffect(() => {
    void refreshStatus();
    let active = true;
    const subscription = listenToWindow<LocalOcrProgress>(
      "ocr:progress",
      ({ payload }) => {
        if (!active || !payload) return;
        setProgress(payload);
        if (payload.stage === "ready") {
          sawReadyRef.current = true;
        }
        if (
          payload.stage === "ready" ||
          payload.stage === "error" ||
          payload.stage === "cancelled"
        ) {
          setCancelling(false);
          void refreshStatus();
        }
      }
    );

    return () => {
      active = false;
      void subscription.then((unlisten) => unlisten()).catch(console.error);
    };
  }, [refreshStatus]);

  const busy =
    Boolean(status?.busy) ||
    actionBusy === "download" ||
    actionBusy === "import";

  const activeModeDesc =
    ENGINE_MODES.find((m) => m.id === engineMode)?.desc ?? ENGINE_MODES[0].desc;

  const handleDownload = async () => {
    if (busy || actionBusy !== null) return;
    setActionBusy("download");
    setCancelling(false);
    setTestOutput(null);
    sawReadyRef.current = false;
    setProgress({
      stage: "downloading",
      downloaded: 0,
      total: null,
      percent: null,
      message: "正在准备下载离线 RapidOCR 引擎…",
    });
    try {
      const next = await invoke<LocalOcrStatus>("download_local_ocr", {
        customUrl: downloadUrl.trim(),
        proxyUrl: proxyUrl.trim(),
      });
      setStatus(next);
      onNotice("ok", `离线 RapidOCR ${next.version} 已安装就绪`);
    } catch (err) {
      const msg = String(err);
      onNotice(msg.includes("已取消") ? "ok" : "err", msg);
      await refreshStatus();
    } finally {
      setActionBusy(null);
      setCancelling(false);
    }
  };

  const handleCancel = async () => {
    if (cancelling) return;
    setCancelling(true);
    try {
      await invoke("cancel_local_ocr_download");
    } catch (err) {
      setCancelling(false);
      onNotice("err", String(err));
    }
  };

  const handleImport = async () => {
    if (busy || actionBusy !== null) return;
    setActionBusy("import");
    setTestOutput(null);
    sawReadyRef.current = false;
    try {
      const next = await invoke<LocalOcrStatus>("import_local_ocr_archive");
      setStatus(next);
      if (sawReadyRef.current) {
        onNotice("ok", `离线 RapidOCR ${next.version} 本地压缩包导入完成`);
      }
    } catch (err) {
      onNotice("err", String(err));
      await refreshStatus();
    } finally {
      setActionBusy(null);
    }
  };

  const handleTest = async () => {
    if (busy || actionBusy !== null) return;
    setActionBusy("test");
    setTestOutput(null);
    try {
      const msg = await invoke<string>("test_local_ocr", { lang: ocrLang });
      setTestOutput(msg);
      onNotice("ok", msg);
    } catch (err) {
      const msg = String(err);
      onNotice("err", msg);
    } finally {
      setActionBusy(null);
    }
  };

  const handleUninstall = async () => {
    if (busy || actionBusy !== null) return;
    setActionBusy("uninstall");
    setTestOutput(null);
    setProgress(null);
    try {
      const next = await invoke<LocalOcrStatus>("uninstall_local_ocr");
      setStatus(next);
      onNotice("ok", "已卸载离线 RapidOCR 引擎并释放磁盘空间");
    } catch (err) {
      onNotice("err", String(err));
      await refreshStatus();
    } finally {
      setActionBusy(null);
    }
  };

  const diskMb = status ? (status.diskBytes / (1024 * 1024)).toFixed(1) : "0.0";
  const percent = progress?.percent ?? null;

  if (IS_MAC) {
    return (
      <section className="card local-ocr-card">
        <div className="card-title">
          <span>本地识字</span>
          <span className="card-subtitle">Apple Vision · 离线运行</span>
        </div>

        <div className="local-ocr-status-box">
          <div className="local-ocr-installed">
            <div className="local-ocr-installed-head">
              <div className="local-ocr-installed-meta">
                <span className="local-ocr-ready-badge">
                  <CheckCircle2 size={13} />
                  已就绪
                </span>
                <span className="local-ocr-disk-meta">
                  中 / 英 / 日 / 韩混排
                </span>
              </div>
              <div className="local-ocr-installed-actions">
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={handleTest}
                  disabled={actionBusy !== null}
                >
                  <Play size={12} />
                  {actionBusy === "test" ? "识别中…" : "测试"}
                </button>
              </div>
            </div>
            {testOutput && (
              <div className="local-ocr-test-result">{testOutput}</div>
            )}
          </div>
        </div>
      </section>
    );
  }

  return (
    <section className="card local-ocr-card">
      <div className="card-title">
        <span>本地识字</span>
        <span className="card-subtitle">RapidOCR 离线引擎</span>
      </div>

      <div className="field">
        <label>识字引擎</label>
        <div className="chip-row local-ocr-mode-row">
          {ENGINE_MODES.map((mode) => (
            <button
              key={mode.id}
              type="button"
              className={`chip ${engineMode === mode.id ? "active" : ""}`}
              onClick={() => onChangeEngineMode(mode.id)}
            >
              {mode.label}
            </button>
          ))}
        </div>
        <small>{activeModeDesc}</small>
      </div>

      <div className="local-ocr-status-box">
        {loadingStatus && !status ? (
          <div className="local-ocr-loading-row">
            <RefreshCw size={14} className="spin" />
            <span>正在检测引擎状态…</span>
          </div>
        ) : busy ? (
          <div className="local-ocr-busy" role="status" aria-live="polite">
            <div className="local-ocr-progress-head">
              <span className="local-ocr-progress-stage">
                {stageLabel(progress)}
              </span>
              <span className="local-ocr-progress-metrics">
                {progress?.stage === "downloading"
                  ? progress.total
                    ? `${formatBytes(progress.downloaded)} / ${formatBytes(progress.total)} · ${percent ?? 0}%`
                    : progress.downloaded > 0
                      ? formatBytes(progress.downloaded)
                      : ""
                  : percent !== null
                    ? `${percent}%`
                    : ""}
              </span>
            </div>
            <div className="local-ocr-progress-track">
              <div
                className={`local-ocr-progress-fill ${percent === null ? "indeterminate" : ""}`}
                style={percent !== null ? { width: `${percent}%` } : undefined}
              />
            </div>
            {actionBusy === "download" && (
              <div className="local-ocr-busy-actions">
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={handleCancel}
                  disabled={cancelling}
                >
                  <X size={13} />
                  {cancelling ? "正在取消…" : "取消"}
                </button>
              </div>
            )}
          </div>
        ) : status?.installed ? (
          <div className="local-ocr-installed">
            <div className="local-ocr-installed-head">
              <div className="local-ocr-installed-meta">
                <span className="local-ocr-ready-badge">
                  <CheckCircle2 size={13} />
                  已就绪 · {status.version}
                </span>
                <span className="local-ocr-disk-meta">
                  占用 {diskMb} MB
                </span>
              </div>
              <div className="local-ocr-installed-actions">
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={handleTest}
                  disabled={actionBusy !== null}
                >
                  <Play size={12} />
                  {actionBusy === "test" ? "识别中…" : "测试"}
                </button>
                <Popconfirm
                  message="卸载离线识字引擎并释放空间？"
                  confirmLabel="卸载"
                  onConfirm={handleUninstall}
                >
                  {(triggerProps) => (
                    <button
                      type="button"
                      className="btn btn-sm local-ocr-danger-btn"
                      disabled={actionBusy !== null}
                      {...triggerProps}
                    >
                      <Trash2 size={12} />
                      {actionBusy === "uninstall" ? "卸载中…" : "卸载"}
                    </button>
                  )}
                </Popconfirm>
              </div>
            </div>

            {status.installDir && (
              <div className="local-ocr-path" title={status.installDir}>
                目录：{status.installDir}
              </div>
            )}

            <div className="local-ocr-langs">
              <div className="local-ocr-langs-label">
                识别语言（保存后生效）
              </div>
              <div className="chip-row local-ocr-lang-row">
                {OCR_LANG_OPTIONS.map((item) => {
                  const available = status.availableLangs.includes(item.id);
                  return (
                    <button
                      key={item.id}
                      type="button"
                      className={`chip ${ocrLang === item.id ? "active" : ""}`}
                      disabled={!available}
                      title={
                        available
                          ? `使用 ${item.label}`
                          : `当前引擎包不含 ${item.label}`
                      }
                      onClick={() => onChangeOcrLang(item.id)}
                    >
                      {item.label}
                    </button>
                  );
                })}
              </div>
            </div>

            {testOutput && (
              <div className="local-ocr-test-result">{testOutput}</div>
            )}
          </div>
        ) : (
          <div className="local-ocr-uninstalled">
            <div className="local-ocr-uninstalled-desc">
              未安装离线引擎（约 70 MB），下载后完全本地运行。
            </div>
            <div className="local-ocr-uninstalled-actions">
              <button
                type="button"
                className="btn btn-primary"
                onClick={handleDownload}
                disabled={actionBusy !== null}
              >
                <Download size={14} />
                下载引擎 (~70 MB)
              </button>
              <button
                type="button"
                className="btn"
                onClick={handleImport}
                disabled={actionBusy !== null}
              >
                <FolderUp size={14} />
                导入本地包
              </button>
            </div>
          </div>
        )}

        {!busy && status?.lastError && (
          <div className="error-box local-ocr-error-box">
            {status.lastError}
          </div>
        )}
      </div>

      <div className="local-ocr-mirror-wrap">
        <button
          type="button"
          className="local-ocr-mirror-toggle"
          aria-expanded={showMirror}
          onClick={() => setShowMirror((v) => !v)}
        >
          {showMirror ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          <span>自定义下载镜像</span>
        </button>

        {showMirror && (
          <div className="field local-ocr-mirror-field">
            <label>镜像地址（可选，留空走官方源）</label>
            <input
              className="input"
              value={downloadUrl}
              onChange={(e) => onChangeDownloadUrl(e.target.value)}
              placeholder="留空使用官方源，或填写 HTTPS .7z 镜像地址"
            />
            <small>
              仅支持 HTTPS；官方源强制校验 SHA-256，并自动应用上方代理。
            </small>
          </div>
        )}
      </div>
    </section>
  );
}
