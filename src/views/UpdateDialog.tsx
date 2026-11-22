import { useEffect } from "react";
import { AlertTriangle, CheckCircle2, Download, RefreshCw, ShieldCheck, Sparkles, X } from "lucide-react";
import { formatBytes } from "../appUpdate";
import type { AppUpdateController } from "../hooks/useAppUpdate";
import { IS_MAC } from "../utils/platform";

function formatPubDate(raw?: string | null): string | null {
  if (!raw) return null;
  const parsed = new Date(raw);
  if (Number.isNaN(parsed.getTime())) return raw;
  return parsed.toLocaleDateString("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  });
}

function renderReleaseNotes(notes?: string | null) {
  const trimmed = notes?.trim();
  if (!trimmed) {
    return (
      <p className="update-notes-empty">
        此版本未附带详细更新说明。下载完成后将使用内置 Luma 官方公钥强制校验更新签名。
      </p>
    );
  }
  const lines = trimmed
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  const allBullets = lines.length > 0 && lines.every((line) => /^[-*•]\s+/.test(line));
  if (allBullets) {
    return (
      <ul className="update-notes-list">
        {lines.map((line, index) => (
          <li key={index}>{line.replace(/^[-*•]\s+/, "")}</li>
        ))}
      </ul>
    );
  }
  return <div className="update-notes-text">{trimmed}</div>;
}

export function UpdateDialog({
  update,
  currentVersion,
}: {
  update: AppUpdateController;
  currentVersion: string;
}) {
  const {
    available,
    checking,
    installing,
    progress,
    dialogOpen,
    statusMessage,
    error,
    dirty,
    closeDialog,
    checkNow,
    installNow,
  } = update;

  useEffect(() => {
    if (!dialogOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !installing) {
        e.preventDefault();
        closeDialog();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [dialogOpen, installing, closeDialog]);

  if (!dialogOpen) return null;

  const publishedAt = formatPubDate(available?.pubDate);
  const percent = progress?.percent ?? null;
  const isDownloading = installing && progress?.stage === "downloading";

  return (
    <div
      className="update-modal-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="update-modal-title"
      onClick={(e) => {
        if (e.target === e.currentTarget && !installing) {
          closeDialog();
        }
      }}
    >
      <div className="update-modal-card">
        <div className="update-modal-header">
          <div className="update-modal-heading">
            <div className="update-modal-icon">
              {available ? <Sparkles size={16} /> : <RefreshCw size={16} className={checking ? "spin" : ""} />}
            </div>
            <div>
              <h2 id="update-modal-title" className="update-modal-title">
                {available ? `发现新版本 Luma v${available.version}` : "检查应用更新"}
              </h2>
              <div className="update-modal-meta">
                {currentVersion && <span>当前 v{currentVersion}</span>}
                {available && (
                  <>
                    {currentVersion && <span>·</span>}
                    <span className="update-source-pill">
                      {available.source === "local" ? "本机签名更新" : "在线发布版本"}
                    </span>
                  </>
                )}
                {publishedAt && (
                  <>
                    <span>·</span>
                    <span>发布于 {publishedAt}</span>
                  </>
                )}
              </div>
            </div>
          </div>
          <button
            type="button"
            className="update-modal-close"
            onClick={closeDialog}
            disabled={installing}
            aria-label="关闭更新窗口"
            title={installing ? "正在更新中" : "关闭"}
          >
            <X size={16} />
          </button>
        </div>

        <div className="update-modal-body">
          {checking && !available ? (
            <div className="update-state-panel">
              <RefreshCw size={18} className="spin" />
              <span>正在连接更新服务器检查最新版本…</span>
            </div>
          ) : available ? (
            <>
              <div className="update-notes-box">
                <div className="update-notes-label">更新内容</div>
                {renderReleaseNotes(available.notes)}
              </div>

              <div className="update-hint-row">
                <ShieldCheck size={14} />
                <span>
                  {IS_MAC
                    ? "将打开 GitHub Release 下载 macOS 安装镜像 (.dmg)，下载后拖入「应用程序」覆盖即可，配置和历史记录会保留。"
                    : "安装前会验证官方更新签名。安装时 Luma 将自动重启，暂存截图与贴图会关闭，配置和历史记录会保留。"}
                </span>
              </div>
            </>
          ) : statusMessage ? (
            <div className="update-state-panel success">
              <CheckCircle2 size={18} />
              <span>{statusMessage}</span>
            </div>
          ) : null}

          {dirty && available && !installing && (
            <div className="update-warn-banner">
              <AlertTriangle size={14} />
              <span>检测到未保存的设置，请先关闭此弹窗并保存设置后再安装更新。</span>
            </div>
          )}

          {installing && progress && (
            <div className="update-progress-box" role="status" aria-live="polite">
              <div className="update-progress-head">
                <span className="update-progress-stage">
                  {progress.message || "正在处理更新…"}
                </span>
                <span className="update-progress-metrics">
                  {isDownloading
                    ? progress.total
                      ? `${formatBytes(progress.downloaded)} / ${formatBytes(progress.total)} · ${percent ?? 0}%`
                      : formatBytes(progress.downloaded)
                    : percent !== null
                      ? `${percent}%`
                      : ""}
                </span>
              </div>
              <div className="update-progress-track">
                <div
                  className={`update-progress-fill ${percent === null ? "indeterminate" : ""}`}
                  style={percent !== null ? { width: `${percent}%` } : undefined}
                />
              </div>
            </div>
          )}

          {error && (
            <div className="error-box" style={{ marginTop: 0 }}>
              {error}
            </div>
          )}
        </div>

        <div className="update-modal-footer">
          <button
            type="button"
            className="btn"
            onClick={() => void checkNow({ openDialog: true })}
            disabled={checking || installing}
          >
            <RefreshCw size={13} className={checking ? "spin" : ""} />
            {checking ? "检查中…" : "重新检查"}
          </button>

          <div className="update-modal-footer-right">
            <button
              type="button"
              className="btn"
              onClick={closeDialog}
              disabled={installing}
            >
              {available ? "稍后再说" : "关闭"}
            </button>
            {available && (
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => void installNow()}
                disabled={checking || installing || dirty}
              >
                <Download size={14} />
                {installing
                  ? progress?.stage === "verifying"
                    ? "校验签名中…"
                    : progress?.stage === "installing"
                      ? "正在启动安装…"
                      : percent !== null
                        ? `下载中 ${percent}%`
                        : "准备下载…"
                  : IS_MAC
                    ? `前往下载 v${available.version} (.dmg)`
                    : `下载并安装 v${available.version}`}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
