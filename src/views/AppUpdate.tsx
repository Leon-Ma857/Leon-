import { formatBytes } from "../appUpdate";
import type { AppUpdateController } from "../hooks/useAppUpdate";

export function AppUpdate({ update }: { update: AppUpdateController }) {
  const {
    available,
    checking,
    installing,
    progress,
    statusMessage,
    error,
    dirty,
    openDialog,
    checkNow,
  } = update;
  const busy = checking || installing;

  let description = "启动后及驻留托盘时自动检查新版，安装前强制验证 Luma 官方签名。";
  if (installing && progress) {
    if (progress.stage === "downloading") {
      description = progress.total
        ? `正在下载 Luma v${progress.version}：${formatBytes(progress.downloaded)} / ${formatBytes(progress.total)}（${progress.percent ?? 0}%）`
        : `正在下载 Luma v${progress.version}：${formatBytes(progress.downloaded)}`;
    } else {
      description = progress.message || "正在安装更新…";
    }
  } else if (checking) {
    description = "正在连接更新服务器检查最新版本…";
  } else if (error) {
    description = error;
  } else if (available) {
    description = dirty
      ? `发现新版 Luma v${available.version}，保存设置后即可安装。`
      : available.source === "local"
        ? `本机更新 Luma v${available.version} 已通过签名验证，可查看详情并安装。`
        : `发现新版 Luma v${available.version}，可查看更新说明并下载安装。`;
  } else if (statusMessage) {
    description = statusMessage;
  }

  return (
    <div className="switch-row">
      <div style={{ flex: 1, minWidth: 0 }}>
        <div className="switch-label">
          应用更新
          {available && !installing && (
            <span className="update-inline-badge">v{available.version}</span>
          )}
        </div>
        <div className="switch-desc" role="status">
          {description}
        </div>
        {installing && progress && (
          <div className="update-progress-track compact" style={{ marginTop: 6 }}>
            <div
              className={`update-progress-fill ${progress.percent == null ? "indeterminate" : ""}`}
              style={progress.percent != null ? { width: `${progress.percent}%` } : undefined}
            />
          </div>
        )}
      </div>
      <div className="chip-row" style={{ marginTop: 0, flexShrink: 0 }}>
        <button
          type="button"
          className="btn btn-sm"
          disabled={busy}
          onClick={() => void checkNow({ openDialog: true })}
        >
          {checking ? "检查中…" : "检查更新"}
        </button>
        {(available || installing) && (
          <button
            type="button"
            className="btn btn-sm btn-primary"
            onClick={openDialog}
          >
            {installing ? "查看进度" : "查看并安装"}
          </button>
        )}
      </div>
    </div>
  );
}
