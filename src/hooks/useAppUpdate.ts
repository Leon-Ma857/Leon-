import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  checkAppUpdate,
  getAvailableAppUpdate,
  installAppUpdate,
  noNewerUpdate,
  type AvailableUpdate,
  type UpdateProgress,
} from "../appUpdate";
import { listenToWindow } from "../utils/windowEvents";

export interface AppUpdateController {
  available: AvailableUpdate | null;
  checking: boolean;
  installing: boolean;
  progress: UpdateProgress | null;
  dialogOpen: boolean;
  statusMessage: string | null;
  error: string | null;
  dirty: boolean;
  openDialog: () => void;
  closeDialog: () => void;
  checkNow: (options?: { openDialog?: boolean }) => Promise<void>;
  installNow: () => Promise<void>;
}

export function useAppUpdate({ dirty }: { dirty: boolean }): AppUpdateController {
  const [available, setAvailable] = useState<AvailableUpdate | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const availableRef = useRef<AvailableUpdate | null>(null);
  availableRef.current = available;
  const checkingRef = useRef(false);
  const installingRef = useRef(false);
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;

  const refreshAvailable = useCallback(() => {
    void getAvailableAppUpdate()
      .then((release) => {
        setAvailable(release);
      })
      .catch(console.error);
  }, []);

  const openDialog = useCallback(() => {
    if (!installingRef.current) {
      setError(null);
    }
    setDialogOpen(true);
  }, []);

  const closeDialog = useCallback(() => {
    if (installingRef.current) return;
    setDialogOpen(false);
  }, []);

  const checkNow = useCallback(async (options?: { openDialog?: boolean }) => {
    if (checkingRef.current || installingRef.current) {
      if (options?.openDialog) setDialogOpen(true);
      return;
    }
    const shouldOpen = options?.openDialog ?? true;
    checkingRef.current = true;
    setChecking(true);
    setError(null);
    setStatusMessage(null);
    setProgress(null);
    if (shouldOpen) {
      setDialogOpen(true);
    }
    try {
      const release = await checkAppUpdate();
      setAvailable(release);
      if (!release) {
        setStatusMessage(noNewerUpdate);
      } else {
        setStatusMessage(null);
        if (shouldOpen) {
          setDialogOpen(true);
        }
      }
    } catch (err) {
      setError(String(err));
      if (shouldOpen) {
        setDialogOpen(true);
      }
    } finally {
      checkingRef.current = false;
      setChecking(false);
    }
  }, []);

  const installNow = useCallback(async () => {
    const release = availableRef.current;
    if (!release || dirtyRef.current || installingRef.current || checkingRef.current) {
      return;
    }
    installingRef.current = true;
    setInstalling(true);
    setError(null);
    setStatusMessage(null);
    setDialogOpen(true);
    setProgress({
      stage: "checking",
      version: release.version,
      downloaded: 0,
      total: null,
      percent: null,
      message: "正在核对更新版本…",
    });
    try {
      await installAppUpdate(release);
    } catch (err) {
      setError(String(err));
      setProgress(null);
      installingRef.current = false;
      setInstalling(false);
      refreshAvailable();
    }
  }, [refreshAvailable]);

  const checkRef = useRef(checkNow);
  checkRef.current = checkNow;

  useEffect(() => {
    let active = true;
    let request = 0;

    const refresh = () => {
      const current = ++request;
      void getAvailableAppUpdate()
        .then((release) => {
          if (active && current === request) {
            setAvailable(release);
          }
        })
        .catch(console.error);
    };

    const subscriptions = [
      listenToWindow("update:available", refresh),
      listenToWindow<UpdateProgress>("update:progress", ({ payload }) => {
        if (!active || !payload) return;
        setProgress(payload);
        if (payload.stage === "error" && payload.message) {
          setError(payload.message);
        }
      }),
      listen("open-update", () => {
        if (!active) return;
        if (availableRef.current || installingRef.current) {
          setDialogOpen(true);
        } else {
          void checkRef.current({ openDialog: true });
        }
      }),
    ];

    window.addEventListener("focus", refresh);
    refresh();

    return () => {
      active = false;
      window.removeEventListener("focus", refresh);
      for (const subscription of subscriptions) {
        void subscription.then((unlisten) => unlisten()).catch(console.error);
      }
    };
  }, []);

  return {
    available,
    checking,
    installing,
    progress,
    dialogOpen,
    statusMessage,
    error,
    dirty,
    openDialog,
    closeDialog,
    checkNow,
    installNow,
  };
}
