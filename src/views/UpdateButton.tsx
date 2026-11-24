import { ChevronRight, RefreshCw } from "lucide-react";
import type { AppUpdateController } from "../hooks/useAppUpdate";

export function UpdateButton({
  update,
  variant = "default",
}: {
  update: AppUpdateController;
  variant?: "default" | "sidebar";
}) {
  const { available, checking, installing, progress, openDialog, checkNow } = update;
  const busy = checking || installing;

  const label = installing
    ? progress?.stage === "downloading" && progress.percent != null
      ? `下载 ${progress.percent}%`
      : progress?.stage === "verifying"
        ? "校验签名…"
        : "安装更新…"
    : checking
      ? "检查更新…"
      : available
        ? `发现 v${available.version}`
        : "检查更新";

  const handleClick = () => {
    if (available || installing) {
      openDialog();
    } else {
      void checkNow({ openDialog: true });
    }
  };

  if (variant === "sidebar") {
    return (
      <button
        type="button"
        className={`sidebar-update-btn${available && !busy ? " is-available" : ""}`}
        onClick={handleClick}
        disabled={checking}
        title={available ? `查看 Luma v${available.version} 更新内容并安装` : "检查并安装更新"}
      >
        <span className="sidebar-update-left">
          {available && !busy ? (
            <span className="update-dot" aria-hidden="true" />
          ) : (
            <RefreshCw size={13} className={busy ? "spin" : ""} />
          )}
          <span>{label}</span>
        </span>
        <ChevronRight size={13} className="sidebar-update-chevron" />
      </button>
    );
  }

  return (
    <button
      type="button"
      className={`btn ${available && !busy ? "btn-update-ready" : ""}`}
      onClick={handleClick}
      disabled={checking}
      title={available ? `查看 Luma v${available.version} 更新内容并安装` : "检查并安装更新"}
    >
      {available && !busy && <span className="update-dot" aria-hidden="true" />}
      <RefreshCw size={14} className={busy ? "spin" : ""} />
      {label}
    </button>
  );
}
