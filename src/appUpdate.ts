import { invoke } from "@tauri-apps/api/core";

export type AvailableUpdate = {
  version: string;
  source: "online" | "local";
  notes?: string | null;
  pubDate?: string | null;
};

export type UpdateProgressStage =
  | "checking"
  | "downloading"
  | "verifying"
  | "installing"
  | "error";

export type UpdateProgress = {
  stage: UpdateProgressStage;
  version: string;
  downloaded: number;
  total?: number | null;
  percent?: number | null;
  message?: string | null;
};

export const noNewerUpdate = "当前已是最新版本，暂无可用更新。";

export const getAvailableAppUpdate = () =>
  invoke<AvailableUpdate | null>("get_available_app_update");
export const checkAppUpdate = () =>
  invoke<AvailableUpdate | null>("check_app_update");
export const installAppUpdate = ({ version, source }: AvailableUpdate) =>
  invoke("install_app_update", { expectedVersion: version, source });

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}
