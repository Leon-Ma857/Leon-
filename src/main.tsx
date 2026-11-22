import React, { useEffect } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./styles/global.css";
import "./styles/ask-actions.css";
import "./styles/local-ocr.css";
import { ThemeMode } from "./types";
import { MainSettings } from "./views/MainSettings";
import { OverlayCapture } from "./views/OverlayCapture";
import { PinWindow } from "./views/PinWindow";
import { ResultPopup } from "./views/ResultPopup";
import { StackWindow } from "./views/StackWindow";

const params = new URLSearchParams(window.location.search);
const view = params.get("view") ?? "main";

function applyTheme(theme: ThemeMode | string | undefined) {
  const mode = theme === "light" || theme === "dark" ? theme : "system";
  document.documentElement.dataset.theme = mode;
}

function RootView() {
  useEffect(() => {
    let cancelled = false;
    const syncTheme = async (attempt = 0) => {
      try {
        const theme = await invoke<ThemeMode>("get_theme");
        if (!cancelled) applyTheme(theme);
      } catch {
        if (!cancelled && attempt < 4) {
          setTimeout(() => syncTheme(attempt + 1), 150 * (attempt + 1));
        } else if (!cancelled) {
          applyTheme("system");
        }
      }
    };
    syncTheme();

    let unlisten: (() => void) | undefined;
    listen<string>("theme:updated", (event) => {
      applyTheme(event.payload);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  if (view === "overlay") {
    return <OverlayCapture />;
  }
  if (view === "result") {
    return <ResultPopup />;
  }
  if (view === "stack") {
    return <StackWindow />;
  }
  if (view === "pin") {
    return <PinWindow />;
  }
  return <MainSettings />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <RootView />
  </React.StrictMode>
);
