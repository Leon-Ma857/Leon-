import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LogicalSize, PhysicalPosition, PhysicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { currentMonitor } from "@tauri-apps/api/window";
import {
  Check,
  Copy,
  Download,
  FlipHorizontal,
  Languages,
  Pin,
  PinOff,
  RotateCw,
  ScanText,
  Sparkles,
  X,
} from "lucide-react";
import { PinItemDto } from "../types";

const OPACITY_LEVELS = [1, 0.78, 0.52];

export function PinWindow() {
  const [item, setItem] = useState<PinItemDto | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pinnedTop, setPinnedTop] = useState(true);
  const [opacityIdx, setOpacityIdx] = useState(0);
  const [scale, setScale] = useState(1);
  const [rotation, setRotation] = useState(0);
  const [flippedX, setFlippedX] = useState(false);
  const [copied, setCopied] = useState(false);
  const [hudWidth, setHudWidth] = useState(
    () => (typeof window !== "undefined" ? window.innerWidth : 480)
  );
  const compactHud = hudWidth < 460;
  const ultraCompactHud = hudWidth < 340;

  const params = new URLSearchParams(window.location.search);
  const pinId = Number(params.get("pinId") ?? "0");

  useEffect(() => {
    const onResize = () => setHudWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  const handleClose = useCallback(async () => {
    if (!pinId) {
      await getCurrentWebviewWindow().close();
      return;
    }
    await invoke("close_pin_window", { pinId }).catch(console.error);
  }, [pinId]);

  useEffect(() => {
    if (!pinId) {
      setError("无效的贴图 ID");
      return;
    }
    invoke<PinItemDto>("get_pin_item", { pinId })
      .then((data) => setItem(data))
      .catch((err) => setError(String(err)));

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        handleClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [pinId, handleClose]);

  const applyPinWindowSize = async (
    targetItem: PinItemDto,
    nextScale: number,
    nextRotation: number
  ) => {
    const sf = Math.max(targetItem.scaleFactor || 1, 1);
    const rawW = Math.max(140, Math.min(920, targetItem.pixelWidth / sf));
    const rawH = Math.max(48, Math.min(720, targetItem.pixelHeight / sf));
    const isOddRot = nextRotation % 180 !== 0;
    const baseW = isOddRot ? rawH : rawW;
    const baseH = isOddRot ? rawW : rawH;
    const w = Math.max(140, Math.min(1150, Math.round(baseW * nextScale) + 12));
    const h = Math.max(56, Math.min(860, Math.round(baseH * nextScale) + 12));
    const win = getCurrentWebviewWindow();
    const monitor = await currentMonitor();
    if (!monitor) {
      await win.setSize(new LogicalSize(w, h));
      return;
    }
    const { position, size } = monitor.workArea;
    const scale = monitor.scaleFactor;
    const width = Math.min(size.width, Math.round(w * scale));
    const height = Math.min(size.height, Math.round(h * scale));
    const previous = await win.outerPosition();
    const x = Math.max(position.x, Math.min(previous.x, position.x + size.width - width));
    const y = Math.max(position.y, Math.min(previous.y, position.y + size.height - height));
    if (x !== previous.x || y !== previous.y) await win.setPosition(new PhysicalPosition(x, y));
    await win.setSize(new PhysicalSize(width, height));
  };

  const handleWheel = async (e: React.WheelEvent<HTMLDivElement>) => {
    if (!item) return;
    e.preventDefault();
    const factor = e.deltaY < 0 ? 1.1 : 0.9;
    const nextScale = Math.max(0.35, Math.min(2.4, scale * factor));
    setScale(nextScale);
    await applyPinWindowSize(item, nextScale, rotation);
  };

  const handleRotate = async () => {
    if (!item) return;
    const nextRot = (rotation + 90) % 360;
    setRotation(nextRot);
    await applyPinWindowSize(item, scale, nextRot);
  };

  const resetScale = async () => {
    if (!item) return;
    setScale(1);
    setRotation(0);
    setFlippedX(false);
    await applyPinWindowSize(item, 1, 0);
  };

  const toggleTop = async () => {
    const next = !pinnedTop;
    setPinnedTop(next);
    await getCurrentWebviewWindow().setAlwaysOnTop(next);
  };

  const handleCopy = async () => {
    if (!item) return;
    try {
      await invoke("copy_pin_image", { pinId: item.id });
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (err) {
      console.error(err);
    }
  };

  const handleSave = async () => {
    if (!item) return;
    await invoke("save_pin_image", { pinId: item.id }).catch(console.error);
  };

  const handleOpenResult = async (mode: "translate" | "ocr" | "ask") => {
    if (!item) return;
    await invoke("open_pin_in_result", { pinId: item.id, mode }).catch(console.error);
  };

  if (error) {
    return (
      <div className="result-shell">
        <div className="pin-card pin-card-error">
          <div>{error}</div>
          <button type="button" className="hud-icon-btn" onClick={handleClose} style={{ marginTop: 8 }}>
            <X size={13} /> 关闭
          </button>
        </div>
      </div>
    );
  }

  if (!item) return null;

  const opacity = OPACITY_LEVELS[opacityIdx];
  const isOddRotation = rotation % 180 !== 0;
  const imgTransforms: string[] = [];
  if (isOddRotation) imgTransforms.push("translate(-50%, -50%)");
  if (rotation) imgTransforms.push(`rotate(${rotation}deg)`);
  if (flippedX) imgTransforms.push("scaleX(-1)");
  const imgStyle: React.CSSProperties | undefined =
    isOddRotation || imgTransforms.length > 0
      ? {
          ...(isOddRotation
            ? {
                position: "absolute",
                top: "50%",
                left: "50%",
                width: "calc(100vh - 12px)",
                height: "calc(100vw - 12px)",
                maxWidth: "none",
                maxHeight: "none",
              }
            : {}),
          transform: imgTransforms.join(" "),
        }
      : undefined;

  return (
    <div
      className="result-shell"
      onWheel={handleWheel}
      onDoubleClick={(e) => {
        if ((e.target as HTMLElement).closest(".pin-hud, .pin-close-corner")) return;
        handleClose();
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        handleClose();
      }}
    >
      <div className="pin-card" style={{ opacity }}>
        <img
          src={item.imageDataUrl}
          alt={`桌面贴图 #${item.id}`}
          className="pin-img"
          style={imgStyle}
          data-tauri-drag-region
          draggable={false}
        />
        <button
          type="button"
          className="pin-close-corner"
          onClick={handleClose}
          title="关闭贴图（也可双击、右键或按 Esc）"
          aria-label="关闭贴图"
        >
          <X size={13} />
        </button>
        <div className={`pin-hud ${compactHud ? "compact" : ""}`}>
          <button
            type="button"
            className="hud-icon-btn"
            onClick={() => handleOpenResult("translate")}
            title="翻译此贴图"
          >
            <Languages size={13} />
            <span className="pin-btn-label">翻译</span>
          </button>
          <button
            type="button"
            className="hud-icon-btn"
            onClick={() => handleOpenResult("ocr")}
            title="识别文字"
          >
            <ScanText size={13} />
            <span className="pin-btn-label">识字</span>
          </button>
          <button
            type="button"
            className="hud-icon-btn"
            onClick={() => handleOpenResult("ask")}
            title="问 AI"
          >
            <Sparkles size={13} />
            <span className="pin-btn-label">问 AI</span>
          </button>
          <div className="hud-sep" />
          <button
            type="button"
            className="hud-icon-btn"
            onClick={handleCopy}
            title="复制图片"
          >
            {copied ? <Check size={13} /> : <Copy size={13} />}
          </button>
          <button
            type="button"
            className="hud-icon-btn"
            onClick={handleSave}
            title="保存 PNG"
          >
            <Download size={13} />
          </button>
          <button
            type="button"
            className="hud-icon-btn"
            onClick={handleRotate}
            title="顺时针旋转 90°"
          >
            <RotateCw size={13} />
          </button>
          {!ultraCompactHud && (
            <>
              <button
                type="button"
                className={`hud-icon-btn ${flippedX ? "active" : ""}`}
                onClick={() => setFlippedX((f) => !f)}
                title="水平镜像翻转"
              >
                <FlipHorizontal size={13} />
              </button>
              <button
                type="button"
                className="hud-icon-btn"
                onClick={resetScale}
                title="还原 100% 尺寸与角度"
              >
                {Math.round(scale * 100)}%
              </button>
            </>
          )}
          {!compactHud && (
            <button
              type="button"
              className="hud-icon-btn"
              onClick={() => setOpacityIdx((i) => (i + 1) % OPACITY_LEVELS.length)}
              title="切换透明度"
            >
              透{Math.round(opacity * 100)}%
            </button>
          )}
          <button
            type="button"
            className={`hud-icon-btn ${pinnedTop ? "active" : ""}`}
            onClick={toggleTop}
            title={pinnedTop ? "取消置顶" : "窗口置顶"}
          >
            {pinnedTop ? <Pin size={13} /> : <PinOff size={13} />}
          </button>
          <button
            type="button"
            className="hud-icon-btn pin-close-inline"
            onClick={handleClose}
            title="关闭贴图 (Esc)"
          >
            <X size={14} />
          </button>
        </div>
      </div>
    </div>
  );
}
