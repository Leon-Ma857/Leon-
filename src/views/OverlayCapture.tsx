import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listenToWindow as listen } from "../utils/windowEvents";
import { MOD_LABEL } from "../utils/platform";
import {
  ArrowUpRight,
  Check,
  ChevronsDown,
  Circle,
  Copy,
  Download,
  ExternalLink,
  Grid,
  Languages,
  Layers,
  Loader2,
  MousePointer,
  Pencil,
  Pin,
  PlusSquare,
  QrCode,
  RotateCcw,
  ScanText,
  Send,
  Sparkles,
  Square,
  Trash2,
  Type,
  Undo2,
  X,
} from "lucide-react";
import {
  ENGINE_OPTIONS,
  OverlayAnalysisState,
  OverlayPayload,
  RegionInput,
  StageRegionsOutcome,
  StreamPreviewEvent,
  TARGET_LANGUAGES,
  TranslateEngine,
  TranslatedBlock,
} from "../types";
import {
  ComputedBlockLayout,
  computeInPlaceBlockLayouts,
  createThemeSampleContext as createBlockThemeSampleContext,
  wrapCanvasText,
} from "../utils/inPlaceLayout";
import { AskQuickActionsBar } from "../components/AskQuickActionsBar";

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

type ToolKind = "select" | "rect" | "ellipse" | "arrow" | "pen" | "mosaic" | "text";
type HandleKind = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

interface Point {
  x: number;
  y: number;
}

type AnnotationItem =
  | {
      kind: "rect" | "ellipse";
      color: string;
      width: number;
      x1: number;
      y1: number;
      x2: number;
      y2: number;
    }
  | {
      kind: "arrow";
      color: string;
      width: number;
      x1: number;
      y1: number;
      x2: number;
      y2: number;
    }
  | {
      kind: "pen";
      color: string;
      width: number;
      points: Point[];
    }
  | {
      kind: "mosaic";
      blockSize: number;
      points: Point[];
    }
  | {
      kind: "text";
      color: string;
      fontSize: number;
      x: number;
      y: number;
      text: string;
    };

interface RegionEntry {
  id: number;
  rect: Rect;
  annotations: AnnotationItem[];
}

const COLORS = ["#ef4444", "#f59e0b", "#0071e3", "#10b981", "#ffffff"];
const STROKE_WIDTHS = [2, 4, 7];

function getRegionsSignature(list: RegionEntry[]): string {
  return list
    .map(
      (r) =>
        `${r.id}:${Math.round(r.rect.x)},${Math.round(r.rect.y)},${Math.round(r.rect.w)},${Math.round(r.rect.h)}:${JSON.stringify(r.annotations)}`
    )
    .join("|");
}

function clampRect(r: Rect, vw: number, vh: number): Rect {
  const x = Math.max(0, Math.min(r.x, vw - 10));
  const y = Math.max(0, Math.min(r.y, vh - 10));
  const w = Math.max(10, Math.min(r.w, vw - x));
  const h = Math.max(10, Math.min(r.h, vh - y));
  return { x, y, w, h };
}

function drawArrow(
  ctx: CanvasRenderingContext2D,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
  color: string,
  lineWidth: number
) {
  const headLen = Math.max(12, lineWidth * 3.8);
  const angle = Math.atan2(y2 - y1, x2 - x1);
  const shaftEndX = x2 - Math.cos(angle) * (headLen * 0.65);
  const shaftEndY = y2 - Math.sin(angle) * (headLen * 0.65);

  ctx.save();
  ctx.strokeStyle = color;
  ctx.fillStyle = color;
  ctx.lineWidth = lineWidth;
  ctx.lineCap = "round";

  ctx.beginPath();
  ctx.moveTo(x1, y1);
  ctx.lineTo(shaftEndX, shaftEndY);
  ctx.stroke();

  ctx.beginPath();
  ctx.moveTo(x2, y2);
  ctx.lineTo(
    x2 - headLen * Math.cos(angle - Math.PI / 7),
    y2 - headLen * Math.sin(angle - Math.PI / 7)
  );
  ctx.lineTo(
    x2 - headLen * Math.cos(angle + Math.PI / 7),
    y2 - headLen * Math.sin(angle + Math.PI / 7)
  );
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

function renderAnnotationItem(
  ctx: CanvasRenderingContext2D,
  item: AnnotationItem,
  mosaicSampleCtx: CanvasRenderingContext2D | null,
  scaleX: number,
  scaleY: number
) {
  if (item.kind === "rect") {
    ctx.save();
    ctx.strokeStyle = item.color;
    ctx.lineWidth = item.width;
    ctx.lineJoin = "round";
    const x = Math.min(item.x1, item.x2);
    const y = Math.min(item.y1, item.y2);
    const w = Math.abs(item.x2 - item.x1);
    const h = Math.abs(item.y2 - item.y1);
    ctx.strokeRect(x, y, w, h);
    ctx.restore();
    return;
  }

  if (item.kind === "ellipse") {
    ctx.save();
    ctx.strokeStyle = item.color;
    ctx.lineWidth = item.width;
    const cx = (item.x1 + item.x2) / 2;
    const cy = (item.y1 + item.y2) / 2;
    const rx = Math.abs(item.x2 - item.x1) / 2;
    const ry = Math.abs(item.y2 - item.y1) / 2;
    ctx.beginPath();
    ctx.ellipse(cx, cy, Math.max(rx, 1), Math.max(ry, 1), 0, 0, Math.PI * 2);
    ctx.stroke();
    ctx.restore();
    return;
  }

  if (item.kind === "arrow") {
    drawArrow(ctx, item.x1, item.y1, item.x2, item.y2, item.color, item.width);
    return;
  }

  if (item.kind === "pen") {
    if (item.points.length < 2) return;
    ctx.save();
    ctx.strokeStyle = item.color;
    ctx.lineWidth = item.width;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.beginPath();
    ctx.moveTo(item.points[0].x, item.points[0].y);
    for (let i = 1; i < item.points.length; i++) {
      ctx.lineTo(item.points[i].x, item.points[i].y);
    }
    ctx.stroke();
    ctx.restore();
    return;
  }

  if (item.kind === "text") {
    if (!item.text.trim()) return;
    ctx.save();
    ctx.fillStyle = item.color;
    ctx.font = `600 ${item.fontSize}px "SF Pro Text", "Segoe UI", "PingFang SC", sans-serif`;
    ctx.textBaseline = "top";
    ctx.shadowColor = "rgba(0,0,0,0.45)";
    ctx.shadowBlur = 4;
    const lines = item.text.split("\n");
    lines.forEach((line, idx) => {
      ctx.fillText(line, item.x, item.y + idx * item.fontSize * 1.32);
    });
    ctx.restore();
    return;
  }

  if (item.kind === "mosaic") {
    const b = item.blockSize;
    const brushR = b * 1.15;
    const painted = new Set<string>();
    for (const pt of item.points) {
      const startGx = Math.floor((pt.x - brushR) / b) * b;
      const endGx = Math.floor((pt.x + brushR) / b) * b;
      const startGy = Math.floor((pt.y - brushR) / b) * b;
      const endGy = Math.floor((pt.y + brushR) / b) * b;

      for (let gx = startGx; gx <= endGx; gx += b) {
        for (let gy = startGy; gy <= endGy; gy += b) {
          const key = `${gx},${gy}`;
          if (painted.has(key)) continue;
          painted.add(key);

          let fill = "rgba(30, 41, 59, 0.85)";
          if (mosaicSampleCtx) {
            const sx = Math.max(
              0,
              Math.min(
                mosaicSampleCtx.canvas.width - 1,
                Math.round((gx + b / 2) * scaleX)
              )
            );
            const sy = Math.max(
              0,
              Math.min(
                mosaicSampleCtx.canvas.height - 1,
                Math.round((gy + b / 2) * scaleY)
              )
            );
            const pixel = mosaicSampleCtx.getImageData(sx, sy, 1, 1).data;
            fill = `rgb(${pixel[0]}, ${pixel[1]}, ${pixel[2]})`;
          }
          ctx.fillStyle = fill;
          ctx.fillRect(gx, gy, b + 0.5, b + 0.5);
        }
      }
    }
  }
}

const THEME_SAMPLE_SIZE = 7;

function createThemeSampleContext() {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = THEME_SAMPLE_SIZE;
  return canvas.getContext("2d", { willReadFrequently: true });
}

function sampleRegionTheme(
  sampleCtx: CanvasRenderingContext2D | null,
  image: HTMLImageElement,
  sx: number,
  sy: number,
  sw: number,
  sh: number
): { bgColor: string; textColor: string } {
  if (!sampleCtx) return { bgColor: "rgba(15, 23, 42, 0.92)", textColor: "#f8fafc" };
  // Sample the edge color from a tiny downscaled crop, not a full-screen canvas.
  sampleCtx.clearRect(0, 0, THEME_SAMPLE_SIZE, THEME_SAMPLE_SIZE);
  const size = THEME_SAMPLE_SIZE;
  sampleCtx.drawImage(image, sx, sy, sw, sh, 0, 0, size, size);
  const pixels = sampleCtx.getImageData(0, 0, size, size).data;
  let rSum = 0;
  let gSum = 0;
  let bSum = 0;
  let count = 0;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      if (x !== 0 && x !== size - 1 && y !== 0 && y !== size - 1) continue;
      const index = (y * size + x) * 4;
      rSum += pixels[index];
      gSum += pixels[index + 1];
      bSum += pixels[index + 2];
      count++;
    }
  }
  const r = Math.round(rSum / Math.max(count, 1));
  const g = Math.round(gSum / Math.max(count, 1));
  const b = Math.round(bSum / Math.max(count, 1));
  const luma = 0.299 * r + 0.587 * g + 0.114 * b;
  return {
    bgColor: `rgb(${r}, ${g}, ${b})`,
    textColor: luma > 145 ? "#18181b" : "#f8fafc",
  };
}

export function OverlayCapture() {
  const [payload, setPayload] = useState<(OverlayPayload & { dataUrl: string }) | null>(null);
  const [regions, setRegions] = useState<RegionEntry[]>([]);
  const [activeRegionId, setActiveRegionId] = useState<number | null>(null);
  const [draftRect, setDraftRect] = useState<Rect | null>(null);
  const [multiAddMode, setMultiAddMode] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [qrResults, setQrResults] = useState<string[] | null>(null);
  const [qrCopiedIdx, setQrCopiedIdx] = useState<number | null>(null);

  const [tool, setTool] = useState<ToolKind>("select");
  const [color, setColor] = useState<string>("#ef4444");
  const [strokeWidth, setStrokeWidth] = useState<number>(4);
  const [draftItem, setDraftItem] = useState<AnnotationItem | null>(null);
  const [textDraft, setTextDraft] = useState<{
    regionId: number;
    x: number;
    y: number;
    value: string;
  } | null>(null);

  const [analysis, setAnalysis] = useState<OverlayAnalysisState>({ status: "idle" });
  const [overlayStream, setOverlayStream] = useState<{
    jobId: number;
    kind: string;
    text: string;
    items: Record<number, { text: string; originalText?: string }>;
  } | null>(null);
  const [analysisSignature, setAnalysisSignature] = useState<string>("");
  const currentSignature = useMemo(() => getRegionsSignature(regions), [regions]);
  const analysisMatchesRegions =
    Boolean(analysisSignature) && analysisSignature === currentSignature;
  const [inPlacePreview, setInPlacePreview] = useState(false);
  const [hoveredBlockKey, setHoveredBlockKey] = useState<string | null>(null);
  const [stagedStatus, setStagedStatus] = useState<{
    count: number;
    maxSize: number;
    latestThumb: string;
    sessionAdded: number;
  } | null>(null);
  const [stackFlying, setStackFlying] = useState<
    {
      id: number;
      x: number;
      y: number;
      w: number;
      h: number;
      delayMs: number;
      dx: number;
      dy: number;
      sx: number;
      sy: number;
      rot: number;
      onLand?: () => void;
    }[]
  >([]);
  const [sidePanelOpen, setSidePanelOpen] = useState(false);
  const [askInput, setAskInput] = useState("");
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [activeEngine, setActiveEngine] = useState<TranslateEngine>("ai");
  const [activeTargetLang, setActiveTargetLang] = useState<string>("简体中文");
  const [cursorPt, setCursorPt] = useState<Point | null>(null);
  const [loupeInfo, setLoupeInfo] = useState<{ hex: string; px: number; py: number } | null>(null);

  const nextIdRef = useRef(1);
  const latestCaptureIdRef = useRef<number>(0);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const loupeCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const mosaicSampleCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const textInputRef = useRef<HTMLTextAreaElement | null>(null);
  const askThreadEndRef = useRef<HTMLDivElement | null>(null);

  const dragModeRef = useRef<
    | { kind: "none" }
    | { kind: "create"; sx: number; sy: number; append: boolean }
    | { kind: "move"; regionId: number; sx: number; sy: number; orig: Rect }
    | { kind: "resize"; regionId: number; handle: HandleKind; sx: number; sy: number; orig: Rect }
    | { kind: "draw"; regionId: number }
  >({ kind: "none" });

  const regionsRef = useRef<RegionEntry[]>([]);
  const activeRegionIdRef = useRef<number | null>(null);
  const sidePanelOpenRef = useRef(false);
  const analysisRef = useRef<OverlayAnalysisState>({ status: "idle" });
  const analysisMatchesRegionsRef = useRef(false);
  const loupeInfoRef = useRef<{ hex: string; px: number; py: number } | null>(null);
  const tabRequestRef = useRef<{
    captureId: number;
    signature: string;
    pendingEvent?: Exclude<OverlayAnalysisState, { status: "idle" }>;
  } | null>(null);
  const lastTabJobIdRef = useRef(0);
  const tabShortcutHeldRef = useRef(false);
  const spaceActionRef = useRef<number | null>(null);
  const stagingRef = useRef(false);
  const stagedStatusRef = useRef<{
    count: number;
    maxSize: number;
    latestThumb: string;
    sessionAdded: number;
  } | null>(null);

  useEffect(() => {
    stagedStatusRef.current = stagedStatus;
  }, [stagedStatus]);

  useEffect(() => {
    regionsRef.current = regions;
  }, [regions]);

  useEffect(() => {
    activeRegionIdRef.current = activeRegionId;
  }, [activeRegionId]);

  useEffect(() => {
    sidePanelOpenRef.current = sidePanelOpen;
  }, [sidePanelOpen]);

  useEffect(() => {
    analysisRef.current = analysis;
  }, [analysis]);

  useEffect(() => {
    analysisMatchesRegionsRef.current = analysisMatchesRegions;
  }, [analysisMatchesRegions]);

  useEffect(() => {
    loupeInfoRef.current = loupeInfo;
  }, [loupeInfo]);

  const activeRegion = useMemo(
    () => regions.find((r) => r.id === activeRegionId) ?? regions[regions.length - 1] ?? null,
    [regions, activeRegionId]
  );

  const clearSelectionState = useCallback(() => {
    dragModeRef.current = { kind: "none" };
    regionsRef.current = [];
    activeRegionIdRef.current = null;
    tabRequestRef.current = null;
    setRegions([]);
    setAnalysisSignature("");
    setActiveRegionId(null);
    setDraftRect(null);
    setMultiAddMode(false);
    stagingRef.current = false;
    setStackFlying([]);
    setBusy(false);
    setNotice(null);
    setQrResults(null);
    setQrCopiedIdx(null);
    setTool("select");
    setDraftItem(null);
    setTextDraft(null);
    setAnalysis({ status: "idle" });
    setInPlacePreview(false);
    setHoveredBlockKey(null);
    setSidePanelOpen(false);
    setAskInput("");
  }, []);

  const resetAll = useCallback(() => {
    clearSelectionState();
    stagedStatusRef.current = null;
    setStagedStatus(null);
    setCursorPt(null);
    setLoupeInfo(null);
  }, [clearSelectionState]);

  const handleCancel = useCallback(async () => {
    if (stagingRef.current) return;
    try {
      // Clear the displayed image only after the native window has hidden.
      await invoke("cancel_capture");
    } catch (err) {
      setNotice(String(err));
    }
  }, []);

  const commitTextDraft = useCallback(() => {
    setTextDraft((current) => {
      if (current && current.value.trim()) {
        setRegions((prev) =>
          prev.map((r) =>
            r.id === current.regionId
              ? {
                  ...r,
                  annotations: [
                    ...r.annotations,
                    {
                      kind: "text",
                      color,
                      fontSize: Math.max(15, strokeWidth * 4 + 8),
                      x: current.x,
                      y: current.y,
                      text: current.value.trim(),
                    },
                  ],
                }
              : r
          )
        );
      }
      return null;
    });
  }, [color, strokeWidth]);

  const getSampleContext = useCallback(() => {
    const img = imgRef.current;
    if (!img?.naturalWidth) return null;
    if (!mosaicSampleCanvasRef.current) {
      const sample = document.createElement("canvas");
      sample.width = img.naturalWidth;
      sample.height = img.naturalHeight;
      const ctx = sample.getContext("2d", { willReadFrequently: true });
      if (!ctx) return null;
      ctx.drawImage(img, 0, 0);
      mosaicSampleCanvasRef.current = sample;
    }
    return mosaicSampleCanvasRef.current.getContext("2d");
  }, []);

  const exportAnnotatedCropBase64 = useCallback(
    (entry: RegionEntry): string | undefined => {
      if (entry.annotations.length === 0) return undefined;
      const img = imgRef.current;
      if (!img || !img.naturalWidth) return undefined;

      const targetRect = entry.rect;
      const vw = Math.max(window.innerWidth, 1);
      const vh = Math.max(window.innerHeight, 1);
      const scaleX = img.naturalWidth / vw;
      const scaleY = img.naturalHeight / vh;

      const sx = Math.max(0, Math.round(targetRect.x * scaleX));
      const sy = Math.max(0, Math.round(targetRect.y * scaleY));
      const sw = Math.max(4, Math.round(targetRect.w * scaleX));
      const sh = Math.max(4, Math.round(targetRect.h * scaleY));

      const out = document.createElement("canvas");
      out.width = sw;
      out.height = sh;
      const ctx = out.getContext("2d");
      if (!ctx) return undefined;

      ctx.drawImage(img, sx, sy, sw, sh, 0, 0, sw, sh);

      ctx.save();
      ctx.scale(scaleX, scaleY);
      ctx.translate(-targetRect.x, -targetRect.y);
      const sampleCtx = entry.annotations.some((item) => item.kind === "mosaic") ? getSampleContext() : null;
      for (const item of entry.annotations) {
        renderAnnotationItem(ctx, item, sampleCtx, scaleX, scaleY);
      }
      ctx.restore();

      return out.toDataURL("image/png");
    },
    [getSampleContext]
  );

  const exportInPlaceTranslatedBase64 = useCallback(
    (
      entry: RegionEntry,
      translatedText: string,
      blocks?: TranslatedBlock[]
    ): string | undefined => {
      const img = imgRef.current;
      if (!img || !img.naturalWidth) return undefined;

      const targetRect = entry.rect;
      const vw = Math.max(window.innerWidth, 1);
      const vh = Math.max(window.innerHeight, 1);
      const scaleX = img.naturalWidth / vw;
      const scaleY = img.naturalHeight / vh;

      const sx = Math.max(0, Math.round(targetRect.x * scaleX));
      const sy = Math.max(0, Math.round(targetRect.y * scaleY));
      const sw = Math.max(4, Math.round(targetRect.w * scaleX));
      const sh = Math.max(4, Math.round(targetRect.h * scaleY));

      const out = document.createElement("canvas");
      out.width = sw;
      out.height = sh;
      const ctx = out.getContext("2d");
      if (!ctx) return undefined;

      ctx.drawImage(img, sx, sy, sw, sh, 0, 0, sw, sh);

      const sampleCtx = getSampleContext();
      if (entry.annotations.length > 0) {
        ctx.save();
        ctx.scale(scaleX, scaleY);
        ctx.translate(-targetRect.x, -targetRect.y);
        for (const item of entry.annotations) {
          renderAnnotationItem(ctx, item, sampleCtx, scaleX, scaleY);
        }
        ctx.restore();
      }

      // Position-aligned per-block translation rendering (preserves icons, borders, and line alignment)
      if (blocks && blocks.length > 0) {
        const blockSampleCtx = createBlockThemeSampleContext();
        const layouts = computeInPlaceBlockLayouts(
          blocks,
          sw,
          sh,
          img,
          sx,
          sy,
          sw,
          sh,
          scaleX,
          blockSampleCtx,
          ctx
        );
        if (layouts.length > 0) {
          ctx.textBaseline = "top";
          for (const lay of layouts) {
            ctx.fillStyle = lay.bgColor;
            ctx.fillRect(lay.patchX, lay.patchY, lay.patchW, lay.patchH);
            ctx.fillStyle = lay.textColor;
            ctx.font = `500 ${lay.fontSize}px "Segoe UI", "Microsoft YaHei", sans-serif`;
            lay.lines.forEach((line, idx) => {
              ctx.fillText(
                line,
                lay.patchX + lay.textOffsetX,
                lay.patchY + lay.textOffsetY + idx * lay.lineHeight,
                lay.maxTextW
              );
            });
          }
          return out.toDataURL("image/png");
        }
      }

      const padX = Math.max(2, Math.round(sw * 0.04));
      const padY = Math.max(2, Math.round(sh * 0.05));
      const maxWidth = sw - padX * 2;
      const maxHeight = sh - padY * 2;
      const minFont = Math.max(8, Math.round(9 * scaleX));
      let fontSize = Math.min(Math.round(28 * scaleX), Math.floor(maxHeight / 1.3));
      let lines: string[] = [];
      let lineHeight = 0;
      for (; fontSize >= minFont; fontSize--) {
        ctx.font = `500 ${fontSize}px "Segoe UI", "Microsoft YaHei", sans-serif`;
        lineHeight = fontSize * 1.3;
        lines = wrapCanvasText(ctx, translatedText.trim(), maxWidth);
        if (lines.length * lineHeight <= maxHeight) break;
      }
      if (fontSize < minFont || maxWidth <= 0) return undefined;

      const theme = sampleRegionTheme(createThemeSampleContext(), img, sx, sy, sw, sh);
      ctx.fillStyle = theme.bgColor;
      ctx.fillRect(0, 0, sw, sh);
      ctx.fillStyle = theme.textColor;
      ctx.textBaseline = "top";
      lines.forEach((line, idx) => ctx.fillText(line, padX, padY + idx * lineHeight, maxWidth));

      return out.toDataURL("image/png");
    },
    [getSampleContext]
  );

  const buildRegionInputs = useCallback((list: RegionEntry[]): RegionInput[] => {
    const vw = Math.max(window.innerWidth, 1);
    const vh = Math.max(window.innerHeight, 1);
    return list
      .filter((entry) => entry.rect.w >= 8 && entry.rect.h >= 8)
      .map((entry, idx) => ({
        rx: entry.rect.x / vw,
        ry: entry.rect.y / vh,
        rw: entry.rect.w / vw,
        rh: entry.rect.h / vh,
        annotatedBase64: exportAnnotatedCropBase64(entry) ?? null,
        label: `选区 ${idx + 1}`,
      }));
  }, [exportAnnotatedCropBase64]);

  const runScanQr = useCallback(
    async (entry?: RegionEntry) => {
      const target =
        entry ??
        regionsRef.current.find((r) => r.id === activeRegionIdRef.current) ??
        regionsRef.current[regionsRef.current.length - 1];
      if (!target || busy || target.rect.w < 8 || target.rect.h < 8) return;
      commitTextDraft();
      const vw = Math.max(window.innerWidth, 1);
      const vh = Math.max(window.innerHeight, 1);
      const captureId = latestCaptureIdRef.current;
      try {
        const res = await invoke<string[]>("scan_qr_selection", {
          captureId,
          rx: target.rect.x / vw,
          ry: target.rect.y / vh,
          rw: target.rect.w / vw,
          rh: target.rect.h / vh,
          annotatedBase64: exportAnnotatedCropBase64(target),
        });
        if (latestCaptureIdRef.current !== captureId) return;
        if (res.length === 0) {
          setQrResults(null);
          setNotice("未检测到二维码");
          setTimeout(() => setNotice(null), 1800);
        } else {
          setQrResults(res);
        }
      } catch (err) {
        if (latestCaptureIdRef.current === captureId) {
          setNotice(String(err));
        }
      }
    },
    [busy, commitTextDraft, exportAnnotatedCropBase64]
  );

  const runScrollCapture = useCallback(
    async (entry?: RegionEntry) => {
      const target =
        entry ??
        regionsRef.current.find((r) => r.id === activeRegionIdRef.current) ??
        regionsRef.current[regionsRef.current.length - 1];
      if (!target || busy || target.rect.w < 24 || target.rect.h < 24) return;
      commitTextDraft();
      setBusy(true);
      const vw = Math.max(window.innerWidth, 1);
      const vh = Math.max(window.innerHeight, 1);
      const captureId = latestCaptureIdRef.current;
      try {
        await invoke("start_scroll_capture", {
          captureId,
          rx: target.rect.x / vw,
          ry: target.rect.y / vh,
          rw: target.rect.w / vw,
          rh: target.rect.h / vh,
        });
        resetAll();
        setPayload(null);
      } catch (err) {
        if (latestCaptureIdRef.current === captureId) {
          setNotice(String(err));
          setBusy(false);
        }
      }
    },
    [busy, commitTextDraft, resetAll]
  );

  const runDirectAction = useCallback(
    async (
      entry: RegionEntry,
      action: "translate" | "ocr" | "ask" | "pin" | "copy" | "save"
    ) => {
      if (busy || entry.rect.w < 8 || entry.rect.h < 8) return;
      commitTextDraft();
      setBusy(true);

      const vw = Math.max(window.innerWidth, 1);
      const vh = Math.max(window.innerHeight, 1);
      const rx = entry.rect.x / vw;
      const ry = entry.rect.y / vh;
      const rw = entry.rect.w / vw;
      const rh = entry.rect.h / vh;
      const annotatedBase64 = exportAnnotatedCropBase64(entry);

      const captureId = latestCaptureIdRef.current;
      try {
        if (action === "copy") {
          await invoke("copy_selection_image", { rx, ry, rw, rh, annotatedBase64, captureId });
          return;
        } else if (action === "save") {
          await invoke<boolean>("save_selection_image", { rx, ry, rw, rh, annotatedBase64, captureId });
          return;
        } else {
          await invoke("submit_selection", {
            rx,
            ry,
            rw,
            rh,
            mode: action,
            annotatedBase64,
          });
        }
        resetAll();
        setPayload(null);
      } catch (err) {
        if (latestCaptureIdRef.current === captureId) setNotice(String(err));
      } finally {
        if (latestCaptureIdRef.current === captureId) setBusy(false);
      }
    },
    [busy, commitTextDraft, exportAnnotatedCropBase64, resetAll]
  );

  const paintFlightCard = useCallback((canvas: HTMLCanvasElement | null, card: Rect) => {
    const img = imgRef.current;
    if (!canvas || !img?.naturalWidth || canvas.dataset.painted) return;
    canvas.dataset.painted = "true";
    const scaleX = img.naturalWidth / Math.max(window.innerWidth, 1);
    const scaleY = img.naturalHeight / Math.max(window.innerHeight, 1);
    // The flying copy is a preview. The original crop is prepared separately at source resolution.
    const scale = Math.min(1, 360 / Math.max(card.w, card.h));
    canvas.width = Math.max(1, Math.round(card.w * scale));
    canvas.height = Math.max(1, Math.round(card.h * scale));
    canvas.getContext("2d")?.drawImage(
      img, card.x * scaleX, card.y * scaleY, card.w * scaleX, card.h * scaleY,
      0, 0, canvas.width, canvas.height,
    );
  }, []);

  const runInPlaceAnalysis = useCallback(
    async (mode: "translate" | "ocr" | "ask", engineOverride?: TranslateEngine, targetLangOverride?: string) => {
      if (busy) return;
      commitTextDraft();
      const currentRegions = regionsRef.current;
      const inputs = buildRegionInputs(currentRegions);
      if (inputs.length === 0) return;
      tabRequestRef.current = null;
      setAnalysisSignature(getRegionsSignature(currentRegions));
      setSidePanelOpen(true);
      setInPlacePreview(false);
      try {
        await invoke("overlay_analyze_regions", {
          captureId: latestCaptureIdRef.current,
          regions: inputs,
          mode,
          engineOverride: engineOverride ?? activeEngine,
          targetLangOverride: targetLangOverride ?? activeTargetLang,
          initialPrompt: null,
        });
      } catch (err) {
        setAnalysis({ status: "idle" });
        setNotice(String(err));
      }
    },
    [activeEngine, activeTargetLang, buildRegionInputs, busy, commitTextDraft]
  );

  const toggleTabInPlaceTranslation = useCallback(async () => {
    if (busy) return;
    const captureId = latestCaptureIdRef.current;
    const currentRegions = regionsRef.current;
    if (!captureId || currentRegions.length === 0) return;
    const signature = getRegionsSignature(currentRegions);
    if (tabRequestRef.current?.captureId === captureId && tabRequestRef.current.signature === signature) return;
    const current = analysisRef.current;
    if (analysisMatchesRegionsRef.current && current.status === "translate") {
      if (current.loading && !tabRequestRef.current) {
        setInPlacePreview(true);
        return;
      }
      if (!current.loading && !current.error && current.items.length > 0) {
        setInPlacePreview((shown) => !shown);
        return;
      }
    }
    const inputs = buildRegionInputs(currentRegions);
    if (!inputs.length) return;
    const request: NonNullable<typeof tabRequestRef.current> = { captureId, signature };
    tabRequestRef.current = request;
    const isCurrent = () => tabRequestRef.current === request && latestCaptureIdRef.current === captureId;
    setNotice(null);
    setAnalysisSignature(signature);
    setSidePanelOpen(false);
    setInPlacePreview(true);
    setAnalysis({
      status: "translate", jobId: 0, thumbs: [], engine: activeEngine, targetLang: activeTargetLang,
      loading: true, items: [], elapsedMs: 0, model: "", error: null,
    });
    try {
      const completion = await invoke<[jobId: number, result: OverlayAnalysisState | null] | null>("overlay_analyze_regions", {
        captureId, regions: inputs, mode: "translate", engineOverride: activeEngine,
        targetLangOverride: activeTargetLang, initialPrompt: null, waitForResult: true,
      });
      if (!isCurrent()) return;
      const [jobId, result] = completion ?? [0, null];
      lastTabJobIdRef.current = Math.max(lastTabJobIdRef.current, jobId);
      if (request.pendingEvent && request.pendingEvent.jobId > jobId) {
        setInPlacePreview(false);
        setSidePanelOpen(true);
        setAnalysis(request.pendingEvent);
        return;
      }
      if (!result || result.status !== "translate" || result.loading) {
        throw new Error("翻译已取消，请重试。");
      }
      setAnalysis(result);
      if (result.error || !result.items.some((item) => item.translatedText?.trim())) {
        setNotice(result.error ?? "没有可显示的译文。");
        setInPlacePreview(false);
      }
    } catch (err) {
      if (!isCurrent()) return;
      setAnalysis({ status: "idle" });
      setInPlacePreview(false);
      setNotice(String(err));
    } finally {
      if (tabRequestRef.current === request) tabRequestRef.current = null;
    }
  }, [activeEngine, activeTargetLang, buildRegionInputs, busy]);

  const runStageToStack = useCallback(async () => {
    if (busy || stagingRef.current) return;
    const captureId = latestCaptureIdRef.current;
    const list = regionsRef.current.filter((r) => r.rect.w >= 8 && r.rect.h >= 8);
    if (!captureId || list.length === 0) return;
    commitTextDraft();
    stagingRef.current = true;
    setBusy(true);

    let land!: () => void;
    const landed = new Promise<void>((resolve) => { land = resolve; });
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const targetCx = vw - 20 - 118;
    const targetCy = vh - 56 - 84;
    const flights = list.map((entry, i) => {
      const cx = entry.rect.x + entry.rect.w / 2;
      const cy = entry.rect.y + entry.rect.h / 2;
      const sx = Math.max(0.18, Math.min(0.85, 132 / Math.max(entry.rect.w, 1)));
      const sy = Math.max(0.18, Math.min(0.85, 76 / Math.max(entry.rect.h, 1)));
      return {
        id: entry.id,
        x: entry.rect.x,
        y: entry.rect.y,
        w: entry.rect.w,
        h: entry.rect.h,
        delayMs: Math.min(i * 45, 180),
        dx: targetCx - cx,
        dy: targetCy - cy,
        sx,
        sy,
        rot: (i % 2 === 0 ? 1 : -1) * (6 + (i % 3) * 3),
        onLand: i === list.length - 1 ? land : undefined,
      };
    });
    setStackFlying(flights);
    try {
      // Let the flying cards paint before exporting any annotated selections.
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      if (latestCaptureIdRef.current !== captureId) return;
      const [, outcome] = await Promise.all([
        landed,
        invoke<StageRegionsOutcome>("stage_regions", {
          captureId,
          regions: buildRegionInputs(list),
        }),
      ]);
      if (latestCaptureIdRef.current === captureId) {
        clearSelectionState();
        invoke("overlay_clear_analysis").catch(() => {});
        setStagedStatus((prev) => ({
          count: outcome.count,
          maxSize: outcome.maxSize,
          latestThumb: outcome.latestThumb,
          sessionAdded: (prev?.sessionAdded ?? 0) + outcome.added,
        }));
      }
    } catch (err) {
      if (latestCaptureIdRef.current !== captureId) return;
      stagingRef.current = false;
      setStackFlying([]);
      setNotice(String(err));
      setBusy(false);
    }
  }, [buildRegionInputs, busy, clearSelectionState, commitTextDraft]);

  const runFinishStackStaging = useCallback(async () => {
    if (busy || stagingRef.current) return;
    const captureId = latestCaptureIdRef.current;
    if (!captureId) return;
    setBusy(true);
    try {
      await invoke("finish_stack_handoff", { captureId });
    } catch (err) {
      if (latestCaptureIdRef.current === captureId) {
        setNotice(String(err));
        setBusy(false);
      }
    }
  }, [busy]);

  // Copy the active selection to the clipboard and close the capture overlay.
  const runCopyAndClose = useCallback(async () => {
    if (busy) return;
    commitTextDraft();
    const list = regionsRef.current;
    const entry =
      list.find((r) => r.id === activeRegionIdRef.current) ?? list[list.length - 1];
    if (!entry || entry.rect.w < 8 || entry.rect.h < 8) return;
    setBusy(true);
    const vw = Math.max(window.innerWidth, 1);
    const vh = Math.max(window.innerHeight, 1);
    const captureId = latestCaptureIdRef.current;
    try {
      await invoke("copy_selection_image", {
        captureId,
        rx: entry.rect.x / vw,
        ry: entry.rect.y / vh,
        rw: entry.rect.w / vw,
        rh: entry.rect.h / vh,
        annotatedBase64: exportAnnotatedCropBase64(entry) ?? null,
      });
    } catch (err) {
      if (latestCaptureIdRef.current === captureId) setNotice(String(err));
    } finally {
      if (latestCaptureIdRef.current === captureId) setBusy(false);
    }
  }, [busy, commitTextDraft, exportAnnotatedCropBase64]);

  const runSpaceAction = useCallback(async () => {
    if (busy || spaceActionRef.current !== null || regionsRef.current.length === 0) return;
    const captureId = latestCaptureIdRef.current;
    spaceActionRef.current = captureId;
    try {
      const hasStagedItems =
        (stagedStatusRef.current?.count ?? 0) > 0 ||
        (await invoke<boolean>("capture_has_staged_items", { captureId }));
      if (latestCaptureIdRef.current !== captureId) return;
      if (hasStagedItems) await runStageToStack();
      else await runCopyAndClose();
    } catch (err) {
      if (latestCaptureIdRef.current === captureId) setNotice(String(err));
    } finally {
      if (spaceActionRef.current === captureId) spaceActionRef.current = null;
    }
  }, [busy, runCopyAndClose, runStageToStack]);

  const runExportTranslated = useCallback(
    async (mode: "copy" | "save" | "pin") => {
      if (busy || !analysisMatchesRegions || analysis.status !== "translate" || analysis.items.length === 0) return;
      const vw = Math.max(window.innerWidth, 1);
      const vh = Math.max(window.innerHeight, 1);

      if (mode === "pin") {
        const inputs: RegionInput[] = [];
        regionsRef.current.forEach((entry, idx) => {
          const item = analysis.items[idx];
          const text = item?.translatedText?.trim() || "";
          const b64 = text
            ? exportInPlaceTranslatedBase64(entry, text, item?.blocks)
            : exportAnnotatedCropBase64(entry);
          inputs.push({
            rx: entry.rect.x / vw,
            ry: entry.rect.y / vh,
            rw: entry.rect.w / vw,
            rh: entry.rect.h / vh,
            annotatedBase64: b64 ?? null,
            label: item?.title ?? `译文 ${idx + 1}`,
          });
        });
        if (inputs.length === 0) return;
        if (inputs.some((input) => !input.annotatedBase64)) {
          setNotice("选区空间不足，无法生成译文图。请扩大选区后重试。");
          return;
        }
        setBusy(true);
        try {
          await invoke("overlay_pin_current", { regions: inputs });
          resetAll();
          setPayload(null);
        } catch (err) {
          setNotice(String(err));
          setBusy(false);
        }
        return;
      }

      const target = activeRegion ?? regionsRef.current[0];
      if (!target) return;
      const targetIdx = regionsRef.current.findIndex((r) => r.id === target.id);
      const item = analysis.items[targetIdx >= 0 ? targetIdx : 0];
      const text = item?.translatedText?.trim() || "";
      const b64 = text
        ? exportInPlaceTranslatedBase64(target, text, item?.blocks)
        : exportAnnotatedCropBase64(target);
      if (!b64) {
        setNotice("选区空间不足，无法生成译文图。请扩大选区后重试。");
        return;
      }
      setBusy(true);
      const captureId = latestCaptureIdRef.current;
      try {
        const rx = target.rect.x / vw;
        const ry = target.rect.y / vh;
        const rw = target.rect.w / vw;
        const rh = target.rect.h / vh;
        if (mode === "copy") {
          await invoke("copy_selection_image", { rx, ry, rw, rh, annotatedBase64: b64, captureId });
        } else {
          await invoke<boolean>("save_selection_image", { rx, ry, rw, rh, annotatedBase64: b64, captureId });
        }
      } catch (err) {
        if (latestCaptureIdRef.current === captureId) setNotice(String(err));
      } finally {
        if (latestCaptureIdRef.current === captureId) setBusy(false);
      }
    },
    [
      activeRegion,
      analysis,
      analysisMatchesRegions,
      busy,
      exportAnnotatedCropBase64,
      exportInPlaceTranslatedBase64,
      resetAll,
    ]
  );

  const runPinAll = useCallback(async () => {
    if (busy) return;
    commitTextDraft();
    const inputs = buildRegionInputs(regionsRef.current);
    if (inputs.length === 0) return;
    setBusy(true);
    try {
      await invoke("overlay_pin_current", { regions: inputs });
      resetAll();
      setPayload(null);
    } catch (err) {
      setNotice(String(err));
      setBusy(false);
    }
  }, [buildRegionInputs, busy, commitTextDraft, resetAll]);

  // Subscribe to backend events once
  useEffect(() => {
    let disposed = false;
    let eventCount = 0;
    const cleanups: (() => void)[] = [];
    const track = (unlisten: () => void) => {
      if (disposed) unlisten();
      else cleanups.push(unlisten);
    };
    let objectUrl: string | undefined;
    const releaseImage = () => {
      if (objectUrl) URL.revokeObjectURL(objectUrl);
      objectUrl = undefined;
      mosaicSampleCanvasRef.current = null;
    };
    const applyCapture = async (data: OverlayPayload) => {
      const generation = eventCount;
      latestCaptureIdRef.current = data.captureId;
      try {
        const png = await invoke<ArrayBuffer>("get_capture_image", { captureId: data.captureId });
        if (disposed || generation !== eventCount) return;
        releaseImage();
        objectUrl = URL.createObjectURL(new Blob([png], { type: "image/png" }));
        resetAll();
        setPayload({ ...data, dataUrl: objectUrl });
        setActiveEngine(data.translateEngine);
        setActiveTargetLang(data.targetLang);
      } catch (err) {
        if (!disposed && generation === eventCount) console.error(err);
      }
    };
    const ready = listen<OverlayPayload>("capture:ready", (event) => {
      if (disposed) return;
      eventCount++;
      applyCapture(event.payload);
    }).then(track);
    const cleared = listen("capture:clear", () => {
      if (disposed) return;
      eventCount++;
      latestCaptureIdRef.current = 0;
      resetAll();
      setPayload(null);
      releaseImage();
    }).then(track);
    Promise.all([ready, cleared]).then(async () => {
      if (disposed) return;
      const before = eventCount;
      const data = await invoke<OverlayPayload | null>("get_current_capture");
      if (!disposed && before === eventCount && data && latestCaptureIdRef.current !== data.captureId) {
        applyCapture(data);
      }
    }).catch(console.error);

    listen<OverlayAnalysisState>("overlay:analysis", (event) => {
      if (disposed) return;
      const next = event.payload;
      if (next.status !== "idle" && next.jobId <= lastTabJobIdRef.current) return;
      const tabRequest = tabRequestRef.current;
      if (tabRequest) {
        const previous = tabRequest.pendingEvent;
        if (next.status !== "idle" && (!previous || next.jobId > previous.jobId ||
            (next.jobId === previous.jobId && !next.loading))) {
          tabRequest.pendingEvent = next;
        }
        return;
      }
      setAnalysis(next);
      if (next.status === "idle" || !next.loading) {
        setOverlayStream(null);
      } else {
        setOverlayStream((cur) => (cur && cur.jobId === next.jobId ? cur : null));
      }
      if (next.status === "idle") {
        setSidePanelOpen(false);
      } else if (next.status === "translate") {
        setActiveEngine(next.engine);
        setActiveTargetLang(next.targetLang);
        if (next.error && !sidePanelOpenRef.current) {
          setNotice(next.error);
          setInPlacePreview(false);
        }
      }
    }).then(track).catch(console.error);

    listen<StreamPreviewEvent>("overlay:stream", (event) => {
      if (disposed) return;
      const p = event.payload;
      setOverlayStream((prev) => {
        const base =
          prev && prev.jobId === p.jobId && prev.kind === p.kind
            ? prev
            : { jobId: p.jobId, kind: p.kind, text: "", items: {} };
        if (p.itemIndex != null) {
          return {
            ...base,
            items: {
              ...base.items,
              [p.itemIndex]: {
                text: p.text,
                originalText: p.originalText ?? undefined,
              },
            },
          };
        }
        return { ...base, text: p.text };
      });
    }).then(track).catch(console.error);

    return () => {
      disposed = true;
      releaseImage();
      cleanups.forEach((unlisten) => unlisten());
    };
  }, [resetAll]);

  useEffect(() => {
    if (analysis.status === "ask") {
      askThreadEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [analysis, overlayStream]);

  const handleImageLoad = () => {
    const img = imgRef.current;
    if (!img || !img.naturalWidth) return;
    const captureId = payload?.captureId ?? 0;
    if (!captureId) return;

    // A hidden WebView can pause animation frames. Wait for decode instead.
    img.decode().then(() => {
      if (latestCaptureIdRef.current === captureId) {
        return invoke("overlay_image_ready", { captureId });
      }
    }).catch(console.error);
  };

  // Repaint annotation canvas across all regions
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    if (canvas.width !== Math.round(vw * dpr) || canvas.height !== Math.round(vh * dpr)) {
      canvas.width = Math.round(vw * dpr);
      canvas.height = Math.round(vh * dpr);
    }

    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, vw, vh);

    const img = imgRef.current;
    const scaleX = img && img.naturalWidth ? img.naturalWidth / vw : 1;
    const scaleY = img && img.naturalHeight ? img.naturalHeight / vh : 1;
    const sampleCtx = tool === "mosaic" ? getSampleContext() : mosaicSampleCanvasRef.current?.getContext("2d") ?? null;

    for (const entry of regions) {
      if (entry.rect.w < 4 || entry.rect.h < 4) continue;
      const hasDraft =
        draftItem &&
        dragModeRef.current.kind === "draw" &&
        dragModeRef.current.regionId === entry.id;
      if (entry.annotations.length === 0 && !hasDraft) continue;

      ctx.save();
      ctx.beginPath();
      ctx.rect(entry.rect.x, entry.rect.y, entry.rect.w, entry.rect.h);
      ctx.clip();

      for (const item of entry.annotations) {
        renderAnnotationItem(ctx, item, sampleCtx, scaleX, scaleY);
      }
      if (hasDraft && draftItem) {
        renderAnnotationItem(ctx, draftItem, sampleCtx, scaleX, scaleY);
      }
      ctx.restore();
    }
  }, [regions, draftItem, tool, getSampleContext]);

  // Keyboard shortcuts
  useEffect(() => {
    const onTabDown = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;
      if (!tabShortcutHeldRef.current) {
        if (e.isComposing || e.shiftKey || e.altKey || e.ctrlKey || e.metaKey || textDraft) return;
        const target = e.target;
        if (target instanceof HTMLElement &&
            (target.isContentEditable || target.closest("input, textarea, select"))) return;
        if (!latestCaptureIdRef.current || regionsRef.current.length === 0) return;
      }
      e.preventDefault();
      e.stopPropagation();
      if (e.repeat || tabShortcutHeldRef.current) return;
      tabShortcutHeldRef.current = true;
      void toggleTabInPlaceTranslation();
    };
    const releaseTab = () => { tabShortcutHeldRef.current = false; };
    const onTabUp = (e: KeyboardEvent) => {
      if (e.key === "Tab") releaseTab();
    };
    window.addEventListener("keydown", onTabDown, true);
    window.addEventListener("keyup", onTabUp, true);
    window.addEventListener("blur", releaseTab);
    return () => {
      window.removeEventListener("keydown", onTabDown, true);
      window.removeEventListener("keyup", onTabUp, true);
      window.removeEventListener("blur", releaseTab);
    };
  }, [textDraft, toggleTabInPlaceTranslation]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.isComposing || e.defaultPrevented) return;
      if (textDraft) {
        if (e.key === "Escape") {
          e.preventDefault();
          setTextDraft(null);
        } else if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          commitTextDraft();
        }
        return;
      }

      const activeEl = document.activeElement;
      if (
        activeEl &&
        (activeEl.tagName === "TEXTAREA" ||
          activeEl.tagName === "INPUT" ||
          activeEl.tagName === "SELECT")
      ) {
        if (e.key === "Escape") {
          e.preventDefault();
          handleCancel();
        }
        return;
      }

      if (e.key === "Escape") {
        e.preventDefault();
        // Esc always leaves the capture. While an analysis panel is open the first press
        // only closes that panel, so a frozen or busy overlay still has a way out.
        if (sidePanelOpen && analysisRef.current.status !== "idle" && !busy) {
          setSidePanelOpen(false);
          invoke("overlay_clear_analysis").catch(console.error);
        } else {
          handleCancel();
        }
        return;
      }

      if (e.key === " " || e.code === "Space") {
        e.preventDefault();
        if (!e.repeat) void runSpaceAction();
        return;
      }

      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
        e.preventDefault();
        const aid = activeRegionIdRef.current;
        if (aid !== null) {
          setRegions((prev) =>
            prev.map((r) =>
              r.id === aid ? { ...r, annotations: r.annotations.slice(0, -1) } : r
            )
          );
        }
        return;
      }

      const currentRegions = regionsRef.current;
      const k = e.key.toLowerCase();
      if (currentRegions.length === 0) {
        if (k === "c" && loupeInfoRef.current?.hex) {
          const hex = loupeInfoRef.current.hex;
          e.preventDefault();
          navigator.clipboard
            .writeText(hex)
            .then(() => {
              setNotice(`已复制色值 ${hex}`);
              setTimeout(() => setNotice(null), 1600);
            })
            .catch(console.error);
        }
        return;
      }

      if (
        tool === "select" &&
        (e.key === "ArrowUp" ||
          e.key === "ArrowDown" ||
          e.key === "ArrowLeft" ||
          e.key === "ArrowRight")
      ) {
        e.preventDefault();
        const targetId = activeRegionIdRef.current ?? currentRegions[currentRegions.length - 1].id;
        const vw = Math.max(window.innerWidth, 1);
        const vh = Math.max(window.innerHeight, 1);
        const dx = e.key === "ArrowLeft" ? -1 : e.key === "ArrowRight" ? 1 : 0;
        const dy = e.key === "ArrowUp" ? -1 : e.key === "ArrowDown" ? 1 : 0;
        setRegions((prev) =>
          prev.map((r) => {
            if (r.id !== targetId) return r;
            if (e.shiftKey) {
              return {
                ...r,
                rect: clampRect(
                  {
                    x: r.rect.x,
                    y: r.rect.y,
                    w: Math.max(10, r.rect.w + dx),
                    h: Math.max(10, r.rect.h + dy),
                  },
                  vw,
                  vh
                ),
              };
            }
            return {
              ...r,
              rect: {
                ...r.rect,
                x: Math.max(0, Math.min(r.rect.x + dx, vw - r.rect.w)),
                y: Math.max(0, Math.min(r.rect.y + dy, vh - r.rect.h)),
              },
            };
          })
        );
        return;
      }

      const targetEntry =
        currentRegions.find((r) => r.id === activeRegionIdRef.current) ??
        currentRegions[currentRegions.length - 1];

      if (e.key === "Enter" || k === "t") {
        e.preventDefault();
        runInPlaceAnalysis("translate");
      } else if (k === "o") {
        e.preventDefault();
        runInPlaceAnalysis("ocr");
      } else if (k === "a") {
        e.preventDefault();
        runInPlaceAnalysis("ask");
      } else if (k === "b") {
        e.preventDefault();
        runStageToStack();
      } else if (k === "p") {
        e.preventDefault();
        runPinAll();
      } else if (k === "c") {
        e.preventDefault();
        runDirectAction(targetEntry, "copy");
      } else if (k === "s") {
        e.preventDefault();
        runDirectAction(targetEntry, "save");
      } else if (k === "q") {
        e.preventDefault();
        void runScanQr(targetEntry);
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    busy,
    commitTextDraft,
    handleCancel,
    runDirectAction,
    runInPlaceAnalysis,
    runPinAll,
    runScanQr,
    runSpaceAction,
    runStageToStack,
    sidePanelOpen,
    textDraft,
    toggleTabInPlaceTranslation,
    tool,
  ]);

  const startHandleResize = (regionId: number, handle: HandleKind, e: React.MouseEvent) => {
    e.stopPropagation();
    const target = regions.find((r) => r.id === regionId);
    if (!target || busy) return;
    setActiveRegionId(regionId);
    dragModeRef.current = {
      kind: "resize",
      regionId,
      handle,
      sx: e.clientX,
      sy: e.clientY,
      orig: { ...target.rect },
    };
  };

  const onMouseDown = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.button === 2) {
      e.preventDefault();
      handleCancel();
      return;
    }
    if (e.button !== 0 || busy) return;
    const el = e.target as HTMLElement;
    if (el.closest(".overlay-hud-wrap")) return;
    if (el.closest(".overlay-side-panel")) return;
    if (el.closest(".overlay-text-input")) return;
    if (el.closest(".region-remove-btn")) return;

    if (textDraft) {
      commitTextDraft();
    }

    // Check from topmost region down whether cursor clicked inside an existing region
    const hit = [...regions]
      .reverse()
      .find(
        (entry) =>
          e.clientX >= entry.rect.x &&
          e.clientX <= entry.rect.x + entry.rect.w &&
          e.clientY >= entry.rect.y &&
          e.clientY <= entry.rect.y + entry.rect.h
      );

    if (hit) {
      setActiveRegionId(hit.id);
      if (tool === "select") {
        dragModeRef.current = {
          kind: "move",
          regionId: hit.id,
          sx: e.clientX,
          sy: e.clientY,
          orig: { ...hit.rect },
        };
        return;
      }

      if (tool === "text") {
        setTextDraft({ regionId: hit.id, x: e.clientX, y: e.clientY, value: "" });
        setTimeout(() => textInputRef.current?.focus(), 0);
        return;
      }

      dragModeRef.current = { kind: "draw", regionId: hit.id };
      const pt = { x: e.clientX, y: e.clientY };
      if (tool === "rect" || tool === "ellipse" || tool === "arrow") {
        setDraftItem({
          kind: tool,
          color,
          width: strokeWidth,
          x1: pt.x,
          y1: pt.y,
          x2: pt.x,
          y2: pt.y,
        });
      } else if (tool === "pen") {
        setDraftItem({
          kind: "pen",
          color,
          width: strokeWidth,
          points: [pt],
        });
      } else if (tool === "mosaic") {
        setDraftItem({
          kind: "mosaic",
          blockSize: strokeWidth * 3 + 6,
          points: [pt],
        });
      }
      return;
    }

    // Start creating a new selection region
    const append = multiAddMode || e.shiftKey;
    if (!append) {
      setRegions([]);
      setActiveRegionId(null);
      setAnalysis({ status: "idle" });
    }
    setDraftRect({ x: e.clientX, y: e.clientY, w: 0, h: 0 });
    dragModeRef.current = { kind: "create", sx: e.clientX, sy: e.clientY, append };
    setDraftItem(null);
    setTool("select");
  };

  const onMouseMove = (e: React.MouseEvent<HTMLDivElement>) => {
    const el = e.target as HTMLElement;
    const overChrome = Boolean(
      el.closest(".overlay-hud-wrap") || el.closest(".overlay-side-panel")
    );
    const mode = dragModeRef.current;
    if (
      !overChrome &&
      tool === "select" &&
      (regionsRef.current.length === 0 || mode.kind === "create" || mode.kind === "resize")
    ) {
      setCursorPt({ x: e.clientX, y: e.clientY });
    } else if (cursorPt !== null) {
      setCursorPt(null);
    }

    if (mode.kind === "none") return;

    const vw = window.innerWidth;
    const vh = window.innerHeight;

    if (mode.kind === "create") {
      setDraftRect({
        x: Math.min(mode.sx, e.clientX),
        y: Math.min(mode.sy, e.clientY),
        w: Math.abs(e.clientX - mode.sx),
        h: Math.abs(e.clientY - mode.sy),
      });
      return;
    }

    if (mode.kind === "move") {
      const dx = e.clientX - mode.sx;
      const dy = e.clientY - mode.sy;
      const moved: Rect = {
        ...mode.orig,
        x: Math.max(0, Math.min(mode.orig.x + dx, vw - mode.orig.w)),
        y: Math.max(0, Math.min(mode.orig.y + dy, vh - mode.orig.h)),
      };
      setRegions((prev) =>
        prev.map((r) => (r.id === mode.regionId ? { ...r, rect: moved } : r))
      );
      return;
    }

    if (mode.kind === "resize") {
      const dx = e.clientX - mode.sx;
      const dy = e.clientY - mode.sy;
      let { x, y, w, h } = mode.orig;
      if (mode.handle.includes("e")) w += dx;
      if (mode.handle.includes("s")) h += dy;
      if (mode.handle.includes("w")) {
        x += dx;
        w -= dx;
      }
      if (mode.handle.includes("n")) {
        y += dy;
        h -= dy;
      }
      if (w < 12) {
        if (mode.handle.includes("w")) x = mode.orig.x + mode.orig.w - 12;
        w = 12;
      }
      if (h < 12) {
        if (mode.handle.includes("n")) y = mode.orig.y + mode.orig.h - 12;
        h = 12;
      }
      const resized = clampRect({ x, y, w, h }, vw, vh);
      setRegions((prev) =>
        prev.map((r) => (r.id === mode.regionId ? { ...r, rect: resized } : r))
      );
      return;
    }

    if (mode.kind === "draw") {
      const pt = { x: e.clientX, y: e.clientY };
      setDraftItem((prev) => {
        if (!prev) return null;
        if (prev.kind === "rect" || prev.kind === "ellipse" || prev.kind === "arrow") {
          return { ...prev, x2: pt.x, y2: pt.y };
        }
        if (prev.kind === "pen" || prev.kind === "mosaic") {
          return { ...prev, points: [...prev.points, pt] };
        }
        return prev;
      });
    }
  };

  const onMouseUp = (e: React.MouseEvent<HTMLDivElement>) => {
    const mode = dragModeRef.current;
    dragModeRef.current = { kind: "none" };
    if (mode.kind === "none") return;

    if (mode.kind === "create") {
      setDraftRect(null);
      const finalRect = {
        x: Math.min(mode.sx, e.clientX),
        y: Math.min(mode.sy, e.clientY),
        w: Math.abs(e.clientX - mode.sx),
        h: Math.abs(e.clientY - mode.sy),
      };
      if (finalRect.w < 10 || finalRect.h < 10) {
        if (!mode.append && regionsRef.current.length === 0) {
          resetAll();
        }
        return;
      }

      const id = nextIdRef.current++;
      const newEntry: RegionEntry = { id, rect: finalRect, annotations: [] };

      if (!mode.append && payload?.autoTranslateOnRelease && regionsRef.current.length === 0) {
        setRegions([newEntry]);
        setActiveRegionId(id);
        runDirectAction(newEntry, "translate");
        return;
      }

      setRegions((prev) => (mode.append ? [...prev, newEntry] : [newEntry]));
      setActiveRegionId(id);
      return;
    }

    if (mode.kind === "draw") {
      const targetId = mode.regionId;
      setDraftItem((current) => {
        if (current) {
          setRegions((prev) =>
            prev.map((r) =>
              r.id === targetId ? { ...r, annotations: [...r.annotations, current] } : r
            )
          );
        }
        return null;
      });
    }
  };

  const removeRegion = (id: number, e: React.MouseEvent) => {
    e.stopPropagation();
    setRegions((prev) => {
      const next = prev.filter((r) => r.id !== id);
      if (activeRegionId === id) {
        setActiveRegionId(next[next.length - 1]?.id ?? null);
      }
      if (next.length === 0) {
        setAnalysis({ status: "idle" });
      }
      return next;
    });
  };

  const switchAnalysisMode = async (
    mode: "translate" | "ocr" | "ask",
    engOverride?: TranslateEngine,
    langOverride?: string
  ) => {
    try {
      commitTextDraft();
      const currentRegions = regionsRef.current;
      const latestInputs = buildRegionInputs(currentRegions);
      if (latestInputs.length === 0) return;
      setAnalysisSignature(getRegionsSignature(currentRegions));
      setInPlacePreview(false);
      await invoke("overlay_switch_mode", {
        mode,
        engineOverride: engOverride ?? activeEngine,
        targetLangOverride: langOverride ?? activeTargetLang,
        regions: latestInputs.length > 0 ? latestInputs : null,
      });
    } catch (err) {
      setNotice(String(err));
    }
  };

  const sendOverlayAsk = async (prompt?: string) => {
    const q = (prompt ?? askInput).trim();
    if (!q || (analysis.status === "ask" && analysis.loading)) return;
    try {
      const list = regionsRef.current;
      const inputs = buildRegionInputs(list);
      if (inputs.length === 0 || inputs.length !== list.length ||
          list.some((region, idx) => region.annotations.length > 0 && !inputs[idx].annotatedBase64)) {
        throw new Error("无法生成当前选区，请检查选区和标注后重试");
      }
      await invoke("overlay_ask_follow_up", { question: q, captureId: latestCaptureIdRef.current, regions: inputs });
      if (!prompt) setAskInput("");
    } catch (err) {
      setNotice(String(err));
    }
  };

  const copyText = async (text: string, key: string) => {
    await navigator.clipboard.writeText(text);
    setCopiedKey(key);
    setTimeout(() => setCopiedKey(null), 1500);
  };

  const scale = payload?.scaleFactor || window.devicePixelRatio || 1;
  const anchorRect = draftRect ?? activeRegion?.rect ?? null;

  const panelW = 396;
  const panelMaxH = Math.min(540, window.innerHeight - 36);
  const panelPos = useMemo(() => {
    if (!activeRegion || !sidePanelOpen || analysis.status === "idle") return null;
    const r = activeRegion.rect;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let left = r.x + r.w + 14;
    if (left + panelW > vw - 14) {
      const tryLeft = r.x - panelW - 14;
      left = tryLeft >= 14 ? tryLeft : Math.max(14, vw - panelW - 14);
    }
    const top = Math.max(14, Math.min(r.y, vh - panelMaxH - 14));
    return { left, top };
  }, [activeRegion, analysis.status, panelMaxH, sidePanelOpen]);

  const hudEstimatedW = 740;
  const showSubbar = regions.length > 0 && tool !== "select";
  const hudEstimatedH = showSubbar ? 84 : 44;

  const { hudTop, hudLeft } = useMemo(() => {
    if (!anchorRect) return { hudTop: 0, hudLeft: 0 };
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const margin = 10;

    let top =
      anchorRect.y + anchorRect.h + 10 + hudEstimatedH <= vh - margin
        ? anchorRect.y + anchorRect.h + 10
        : anchorRect.y - hudEstimatedH - 10 >= 8
          ? anchorRect.y - hudEstimatedH - 10
          : Math.max(8, vh - hudEstimatedH - margin);

    const maxHudX = Math.max(margin, vw - hudEstimatedW - margin);
    let left = Math.max(margin, Math.min(anchorRect.x + anchorRect.w - hudEstimatedW, maxHudX));

    if (panelPos) {
      const panelBottom = panelPos.top + panelMaxH;
      const overlapsVert = top < panelBottom + 8 && top + hudEstimatedH > panelPos.top - 8;
      const overlapsHoriz = left < panelPos.left + panelW + 8 && left + hudEstimatedW > panelPos.left - 8;
      if (overlapsVert && overlapsHoriz) {
        const leftOfPanel = panelPos.left - hudEstimatedW - 12;
        const rightOfPanel = panelPos.left + panelW + 12;
        if (leftOfPanel >= margin) {
          left = leftOfPanel;
        } else if (rightOfPanel + hudEstimatedW <= vw - margin) {
          left = rightOfPanel;
        } else if (anchorRect.y - hudEstimatedH - 10 >= 8) {
          top = anchorRect.y - hudEstimatedH - 10;
        } else if (panelBottom + 10 + hudEstimatedH <= vh - margin) {
          top = panelBottom + 10;
        }
      }
    }

    return { hudTop: top, hudLeft: left };
  }, [anchorRect, hudEstimatedH, panelMaxH, panelPos]);

  const hudAboveSelection = Boolean(anchorRect && hudTop < anchorRect.y);

  const regionThemes = useMemo(() => {
    const imgEl = imgRef.current;
    const map = new Map<number, { bgColor: string; textColor: string }>();
    if (!inPlacePreview || !analysisMatchesRegions || analysis.status !== "translate" || analysis.loading || analysis.error || !imgEl?.naturalWidth) return map;
    const sampleCtx = createThemeSampleContext();
    if (!sampleCtx) return map;
    const vw = Math.max(window.innerWidth, 1);
    const vh = Math.max(window.innerHeight, 1);
    for (const entry of regions) {
      const r = entry.rect;
      map.set(
        entry.id,
        sampleRegionTheme(
          sampleCtx,
          imgEl,
          (r.x / vw) * imgEl.naturalWidth,
          (r.y / vh) * imgEl.naturalHeight,
          (r.w / vw) * imgEl.naturalWidth,
          (r.h / vh) * imgEl.naturalHeight
        )
      );
    }
    return map;
  }, [analysis, analysisMatchesRegions, inPlacePreview, regions]);

  const regionBlockLayouts = useMemo(() => {
    const imgEl = imgRef.current;
    const map = new Map<number, ComputedBlockLayout[]>();
    if (
      !inPlacePreview ||
      !analysisMatchesRegions ||
      analysis.status !== "translate" ||
      analysis.loading ||
      analysis.error ||
      !imgEl?.naturalWidth
    ) {
      return map;
    }
    const sampleCtx = createBlockThemeSampleContext();
    const vw = Math.max(window.innerWidth, 1);
    const vh = Math.max(window.innerHeight, 1);
    regions.forEach((entry, idx) => {
      const blocks = analysis.items[idx]?.blocks;
      if (!blocks || blocks.length === 0) return;
      const r = entry.rect;
      const sx = (r.x / vw) * imgEl.naturalWidth;
      const sy = (r.y / vh) * imgEl.naturalHeight;
      const sw = (r.w / vw) * imgEl.naturalWidth;
      const sh = (r.h / vh) * imgEl.naturalHeight;
      const layouts = computeInPlaceBlockLayouts(
        blocks,
        r.w,
        r.h,
        imgEl,
        sx,
        sy,
        sw,
        sh,
        1,
        sampleCtx
      );
      if (layouts.length > 0) {
        map.set(entry.id, layouts);
      }
    });
    return map;
  }, [analysis, analysisMatchesRegions, inPlacePreview, regions]);

  const showLoupe =
    Boolean(payload?.dataUrl) &&
    Boolean(cursorPt) &&
    tool === "select" &&
    analysis.status === "idle" &&
    !textDraft &&
    (regions.length === 0 ||
      dragModeRef.current.kind === "create" ||
      dragModeRef.current.kind === "resize");

  // Paint pixel loupe canvas & sample HEX color under cursor
  useEffect(() => {
    if (!showLoupe || !cursorPt) {
      setLoupeInfo(null);
      return;
    }
    const sample = imgRef.current;
    const loupe = loupeCanvasRef.current;
    if (!sample?.naturalWidth || !loupe) return;
    const vw = Math.max(window.innerWidth, 1);
    const vh = Math.max(window.innerHeight, 1);
    const px = Math.max(0, Math.min(sample.naturalWidth - 1, Math.round((cursorPt.x / vw) * sample.naturalWidth)));
    const py = Math.max(0, Math.min(sample.naturalHeight - 1, Math.round((cursorPt.y / vh) * sample.naturalHeight)));

    const lctx = loupe.getContext("2d", { willReadFrequently: true });
    if (!lctx) return;

    const patchRadius = 11; // 23x23 source pixels zoomed cleanly into 115x115
    const patchSize = patchRadius * 2 + 1;
    lctx.clearRect(0, 0, loupe.width, loupe.height);
    lctx.fillStyle = "#000000";
    lctx.fillRect(0, 0, loupe.width, loupe.height);
    lctx.imageSmoothingEnabled = false;

    const sx = px - patchRadius;
    const sy = py - patchRadius;
    const srcX = Math.max(0, sx);
    const srcY = Math.max(0, sy);
    const srcRight = Math.min(sample.naturalWidth, sx + patchSize);
    const srcBottom = Math.min(sample.naturalHeight, sy + patchSize);
    const srcW = srcRight - srcX;
    const srcH = srcBottom - srcY;
    if (srcW > 0 && srcH > 0) {
      const cell = loupe.width / patchSize;
      lctx.drawImage(
        sample,
        srcX,
        srcY,
        srcW,
        srcH,
        (srcX - sx) * cell,
        (srcY - sy) * cell,
        srcW * cell,
        srcH * cell
      );
    }

    const center = Math.floor(loupe.width / 2);
    const p = lctx.getImageData(center, center, 1, 1).data;
    const hex = `#${[p[0], p[1], p[2]]
      .map((v) => v.toString(16).padStart(2, "0"))
      .join("")
      .toUpperCase()}`;
    setLoupeInfo({ hex, px, py });
  }, [showLoupe, cursorPt]);

  const cutoutRects: Rect[] = [
    ...regions.map((r) => r.rect),
    ...(draftRect && draftRect.w > 2 && draftRect.h > 2 ? [draftRect] : []),
  ];

  return (
    <div
      className="overlay-root"
      style={{
        cursor: "crosshair",
        background: "transparent",
      }}
      onMouseDown={onMouseDown}
      onMouseMove={onMouseMove}
      onMouseLeave={() => setCursorPt(null)}
      onMouseUp={onMouseUp}
      onDoubleClick={(e) => {
        if (!activeRegion || tool !== "select") return;
        const el = e.target as HTMLElement;
        if (el.closest(".overlay-hud-wrap") || el.closest(".overlay-side-panel")) return;
        const r = activeRegion.rect;
        if (e.clientX >= r.x && e.clientX <= r.x + r.w && e.clientY >= r.y && e.clientY <= r.y + r.h) {
          runDirectAction(activeRegion, "copy");
        }
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        handleCancel();
      }}
    >
      {payload?.dataUrl && (
        <img
          key={payload.captureId}
          ref={imgRef}
          src={payload.dataUrl}
          alt=""
          className="overlay-bg"
          draggable={false}
          onLoad={handleImageLoad}
        />
      )}

      {cutoutRects.length > 0 && (
        <svg className="overlay-dim-svg" width="100%" height="100%">
          <defs>
            <mask id="overlay-cutout-mask">
              <rect width="100%" height="100%" fill="white" />
              {cutoutRects.map((cr, idx) => (
                <rect
                  key={idx}
                  x={cr.x}
                  y={cr.y}
                  width={cr.w}
                  height={cr.h}
                  fill="black"
                />
              ))}
            </mask>
          </defs>
          <rect
            width="100%"
            height="100%"
            fill="rgba(0, 0, 0, 0.16)"
            mask="url(#overlay-cutout-mask)"
          />
        </svg>
      )}

      {cutoutRects.length === 0 && (
        <div className="overlay-hint">
          {stagedStatus
            ? `叠堆 ${stagedStatus.count}/${stagedStatus.maxSize} · 继续框选，Esc / 右键退出`
            : "拖拽框选 · Shift 多选 · C 取色 · Esc 退出"}
        </div>
      )}

      {multiAddMode && regions.length > 0 && !busy && (
        <div className="overlay-mode-banner" onMouseDown={(e) => e.stopPropagation()}>
          <span className="overlay-mode-banner-dot" />
          <span>
            加选模式（已选 {regions.length} 处）· 继续拖拽框选
          </span>
          <button
            type="button"
            className="overlay-mode-banner-btn"
            onClick={() => setMultiAddMode(false)}
          >
            完成
          </button>
        </div>
      )}

      {showLoupe && cursorPt && (
        <div
          className="overlay-loupe"
          style={{
            left:
              cursorPt.x + 148 > window.innerWidth
                ? Math.max(8, cursorPt.x - 134)
                : cursorPt.x + 18,
            top:
              cursorPt.y + 180 > window.innerHeight
                ? Math.max(8, cursorPt.y - 166)
                : cursorPt.y + 18,
          }}
        >
          <div className="overlay-loupe-view">
            <canvas ref={loupeCanvasRef} width={115} height={115} />
            <div className="overlay-loupe-crosshair" />
          </div>
          <div className="overlay-loupe-meta">
            <span
              className="overlay-loupe-swatch"
              style={{ background: loupeInfo?.hex ?? "#000000" }}
            />
            <span className="overlay-loupe-hex">{loupeInfo?.hex ?? "#------"}</span>
            <span className="overlay-loupe-coord">
              {loupeInfo ? `${loupeInfo.px},${loupeInfo.py}` : ""}
            </span>
          </div>
          {regions.length === 0 && (
            <div className="overlay-loupe-tip">按 C 复制色值</div>
          )}
        </div>
      )}

      {notice && (
        <div className="overlay-toast" onClick={() => setNotice(null)}>
          {notice}
        </div>
      )}

      {qrResults && qrResults.length > 0 && (
        <div className="overlay-qr-card" onMouseDown={(e) => e.stopPropagation()}>
          <div className="overlay-qr-head">
            <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
              <QrCode size={14} /> 识别到 {qrResults.length} 个二维码
            </span>
            <button
              type="button"
              className="overlay-mode-banner-btn"
              onClick={() => setQrResults(null)}
            >
              关闭
            </button>
          </div>
          {qrResults.map((code, idx) => {
            const isUrl = /^https?:\/\//i.test(code.trim());
            return (
              <div key={idx} className="overlay-qr-item">
                <div className="overlay-qr-text">{code}</div>
                <div className="overlay-qr-actions">
                  <button
                    type="button"
                    className="overlay-mode-banner-btn"
                    onClick={() => {
                      navigator.clipboard.writeText(code).catch(console.error);
                      setQrCopiedIdx(idx);
                      setTimeout(() => setQrCopiedIdx((cur) => (cur === idx ? null : cur)), 1500);
                    }}
                  >
                    {qrCopiedIdx === idx ? "已复制" : "复制"}
                  </button>
                  {isUrl && (
                    <button
                      type="button"
                      className="overlay-mode-banner-btn"
                      onClick={() => {
                        invoke("open_external_url", { url: code }).catch((err) => setNotice(String(err)));
                      }}
                    >
                      <ExternalLink size={11} style={{ marginRight: 3, verticalAlign: "-1px" }} />
                      打开
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}

      <canvas ref={canvasRef} className="overlay-canvas" />

      {/* Draft selection box while dragging */}
      {draftRect && draftRect.w > 2 && draftRect.h > 2 && (
        <div
          className="overlay-selection active"
          style={{
            left: draftRect.x,
            top: draftRect.y,
            width: draftRect.w,
            height: draftRect.h,
            pointerEvents: "none",
          }}
        >
          <div className="overlay-size-tag">
            {Math.round(draftRect.w * scale)} × {Math.round(draftRect.h * scale)}
          </div>
        </div>
      )}

      {/* Committed selection regions */}
      {regions.map((entry, idx) => {
        const isActive = activeRegion?.id === entry.id;
        const r = entry.rect;
        const pw = Math.round(r.w * scale);
        const ph = Math.round(r.h * scale);
        const edgeInset = Math.min(6, Math.floor(Math.min(r.w, r.h) / 4));
        const handles: { id: HandleKind; style: React.CSSProperties; cursor: string }[] =
          isActive && tool === "select"
            ? [
                {
                  id: "n",
                  style: { top: -4, left: edgeInset, right: edgeInset, height: 8 },
                  cursor: "ns-resize",
                },
                {
                  id: "e",
                  style: { top: edgeInset, right: -4, bottom: edgeInset, width: 8 },
                  cursor: "ew-resize",
                },
                {
                  id: "s",
                  style: { bottom: -4, left: edgeInset, right: edgeInset, height: 8 },
                  cursor: "ns-resize",
                },
                {
                  id: "w",
                  style: { top: edgeInset, left: -4, bottom: edgeInset, width: 8 },
                  cursor: "ew-resize",
                },
                { id: "nw", style: { top: -5, left: -5, width: 11, height: 11 }, cursor: "nwse-resize" },
                { id: "ne", style: { top: -5, right: -5, width: 11, height: 11 }, cursor: "nesw-resize" },
                { id: "se", style: { bottom: -5, right: -5, width: 11, height: 11 }, cursor: "nwse-resize" },
                { id: "sw", style: { bottom: -5, left: -5, width: 11, height: 11 }, cursor: "nesw-resize" },
              ]
            : [];

        const translatedText =
          inPlacePreview &&
          analysisMatchesRegions &&
          analysis.status === "translate" &&
          !analysis.loading &&
          analysis.items[idx]?.translatedText
            ? analysis.items[idx].translatedText
            : null;
        const blockLayouts = translatedText ? regionBlockLayouts.get(entry.id) ?? null : null;
        const theme = translatedText
          ? regionThemes.get(entry.id) ?? {
              bgColor: "rgb(15, 23, 42)",
              textColor: "#f8fafc",
            }
          : null;

        const isAnalyzingThis =
          analysisMatchesRegions &&
          (analysis.status === "translate" || analysis.status === "ocr" || analysis.status === "ask") &&
          analysis.loading;

        return (
          <div
            key={entry.id}
            className={`overlay-selection ${isActive ? "active" : ""}`}
            style={{
              left: r.x,
              top: r.y,
              width: r.w,
              height: r.h,
              opacity: stackFlying.length > 0 ? 0.15 : 1,
              transition: "opacity 0.16s ease",
              pointerEvents: tool === "select" ? "auto" : "none",
              cursor: tool === "select" ? "move" : "crosshair",
            }}
          >
            {blockLayouts && blockLayouts.length > 0 ? (
              <div className="inplace-blocks-layer">
                {blockLayouts.map((lay) => {
                  const isHovered = hoveredBlockKey === `${idx}-${lay.index}`;
                  return (
                    <div
                      key={lay.index}
                      className={`inplace-block-patch ${isHovered ? "highlighted" : ""}`}
                      style={{
                        left: lay.patchX,
                        top: lay.patchY,
                        width: lay.patchW,
                        height: lay.patchH,
                        background: lay.bgColor,
                        color: lay.textColor,
                        paddingLeft: lay.textOffsetX,
                        paddingTop: lay.textOffsetY,
                        fontSize: `${lay.fontSize}px`,
                        lineHeight: `${lay.lineHeight}px`,
                      }}
                    >
                      {lay.lines.map((line, lineIdx) => (
                        <div key={lineIdx} className="inplace-block-line">
                          {line}
                        </div>
                      ))}
                    </div>
                  );
                })}
              </div>
            ) : (
              translatedText &&
              theme && (
                <div
                  className="inplace-translation-fill"
                  style={{
                    background: theme.bgColor,
                    color: theme.textColor,
                    fontSize: `${Math.max(11, Math.min(16, Math.round(r.h * 0.08 + 8)))}px`,
                  }}
                >
                  {translatedText}
                </div>
              )
            )}
            <div
              className="overlay-size-tag"
              style={
                isActive && hudAboveSelection && r.y + r.h + 34 < window.innerHeight
                  ? { top: "auto", bottom: -28 }
                  : r.y < 34
                    ? { top: 6, left: 6 }
                    : undefined
              }
            >
              {regions.length > 1 && <span className="region-num">#{idx + 1}</span>}
              <span>
                {pw} × {ph}
              </span>
              {isAnalyzingThis && (
                <Loader2
                  size={12}
                  className="spin overlay-size-spinner"
                  aria-label={
                    analysis.status === "ocr"
                      ? "正在识别文字"
                      : analysis.status === "ask"
                        ? "AI 正在思考"
                        : "正在翻译"
                  }
                />
              )}
              {regions.length > 1 && (
                <button
                  type="button"
                  className="region-remove-btn"
                  onClick={(e) => removeRegion(entry.id, e)}
                  title="删除此选区"
                >
                  <X size={11} />
                </button>
              )}
            </div>

            {handles.map((h) => (
              <div
                key={h.id}
                className="overlay-handle"
                style={{ ...h.style, cursor: h.cursor }}
                onMouseDown={(e) => startHandleResize(entry.id, h.id, e)}
              />
            ))}
          </div>
        );
      })}

      {textDraft && (
        <textarea
          ref={textInputRef}
          className="overlay-text-input"
          rows={1}
          style={{
            left: textDraft.x,
            top: textDraft.y,
            color,
            fontSize: Math.max(15, strokeWidth * 4 + 8),
          }}
          value={textDraft.value}
          onChange={(e) =>
            setTextDraft((prev) => (prev ? { ...prev, value: e.target.value } : null))
          }
          onBlur={commitTextDraft}
          placeholder="输入标注文字，Enter 确认"
        />
      )}

      {/* Floating Overlay Action Toolbar */}
      {activeRegion && !draftRect && !busy && (
        <div className="overlay-hud-wrap" style={{ top: hudTop, left: hudLeft }}>
          <div className="overlay-toolbar">
            <button
              type="button"
              className={`hud-icon-btn ${tool === "select" ? "active" : ""}`}
              onClick={() => setTool("select")}
              title="移动 / 调整选区"
            >
              <MousePointer size={14} />
            </button>
            <button
              type="button"
              className={`hud-icon-btn ${tool === "rect" ? "active" : ""}`}
              onClick={() => setTool("rect")}
              title="矩形框"
            >
              <Square size={14} />
            </button>
            <button
              type="button"
              className={`hud-icon-btn ${tool === "ellipse" ? "active" : ""}`}
              onClick={() => setTool("ellipse")}
              title="椭圆"
            >
              <Circle size={14} />
            </button>
            <button
              type="button"
              className={`hud-icon-btn ${tool === "arrow" ? "active" : ""}`}
              onClick={() => setTool("arrow")}
              title="箭头"
            >
              <ArrowUpRight size={15} />
            </button>
            <button
              type="button"
              className={`hud-icon-btn ${tool === "pen" ? "active" : ""}`}
              onClick={() => setTool("pen")}
              title="画笔"
            >
              <Pencil size={14} />
            </button>
            <button
              type="button"
              className={`hud-icon-btn ${tool === "mosaic" ? "active" : ""}`}
              onClick={() => setTool("mosaic")}
              title="马赛克遮挡隐私"
            >
              <Grid size={14} />
            </button>
            <button
              type="button"
              className={`hud-icon-btn ${tool === "text" ? "active" : ""}`}
              onClick={() => setTool("text")}
              title="文字标注"
            >
              <Type size={14} />
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              disabled={activeRegion.annotations.length === 0}
              onClick={() =>
                setRegions((prev) =>
                  prev.map((r) =>
                    r.id === activeRegion.id
                      ? { ...r, annotations: r.annotations.slice(0, -1) }
                      : r
                  )
                )
              }
              title={`撤销标注 (${MOD_LABEL}+Z)`}
            >
              <Undo2 size={14} />
            </button>

            <div className="hud-sep" />

            <button
              type="button"
              className={`hud-icon-btn ${multiAddMode ? "active-multi" : ""}`}
              onClick={() => {
                setTool("select");
                setMultiAddMode((v) => !v);
              }}
              title="框选多个区域批量处理"
            >
              <PlusSquare size={14} /> {multiAddMode ? `加选中(${regions.length})` : `加选${regions.length > 1 ? `(${regions.length})` : ""}`}
            </button>

            <div className="hud-sep" />

            <button
              type="button"
              className="hud-icon-btn primary"
              onClick={() => runInPlaceAnalysis("translate")}
              title="翻译 (T / Enter) · Tab 切换原位译文"
            >
              <Languages size={14} /> 翻译
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={() => runInPlaceAnalysis("ocr")}
              title="识字 (O)"
            >
              <ScanText size={14} /> 识字
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={() => runInPlaceAnalysis("ask")}
              title="问 AI (A)"
            >
              <Sparkles size={14} /> 问 AI
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={() => void runScanQr(activeRegion)}
              title="扫码 (Q)"
            >
              <QrCode size={14} /> 扫码
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={() => void runScrollCapture(activeRegion)}
              title="滚动截长图 · Esc 结束"
            >
              <ChevronsDown size={14} /> 截长图
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={runStageToStack}
              title="入叠堆 (B)"
            >
              <Layers size={14} /> 入叠堆
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={runPinAll}
              title="贴图 (P)"
            >
              <Pin size={14} />
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={() => runDirectAction(activeRegion, "save")}
              title="保存 PNG (S)"
            >
              <Download size={14} />
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={() => runDirectAction(activeRegion, "copy")}
              title="复制 (C / 空格)"
            >
              <Copy size={14} />
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={() => {
                clearSelectionState();
                invoke("overlay_clear_analysis").catch(console.error);
              }}
              title="重选"
            >
              <RotateCcw size={13} />
            </button>
            <button
              type="button"
              className="hud-icon-btn"
              onClick={handleCancel}
              title="退出 (Esc)"
            >
              <X size={15} />
            </button>
          </div>

          {showSubbar && (
            <div className="overlay-subbar">
              {tool !== "mosaic" && (
                <>
                  <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    {COLORS.map((c) => (
                      <button
                        key={c}
                        type="button"
                        className={`color-dot ${color === c ? "active" : ""}`}
                        style={{ background: c }}
                        onClick={() => setColor(c)}
                        title="颜色"
                      />
                    ))}
                  </div>
                  <div className="hud-sep" />
                </>
              )}

              <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
                {STROKE_WIDTHS.map((w) => (
                  <button
                    key={w}
                    type="button"
                    className={`size-dot-btn ${strokeWidth === w ? "active" : ""}`}
                    onClick={() => setStrokeWidth(w)}
                    title="粗细"
                  >
                    <span className="size-dot" style={{ width: w + 4, height: w + 4 }} />
                  </button>
                ))}
              </div>

              {activeRegion.annotations.length > 0 && (
                <>
                  <div className="hud-sep" />
                  <button
                    type="button"
                    className="hud-icon-btn"
                    style={{ height: 22, fontSize: 11 }}
                    onClick={() =>
                      setRegions((prev) =>
                        prev.map((r) =>
                          r.id === activeRegion.id ? { ...r, annotations: [] } : r
                        )
                      )
                    }
                    title="清空当前选区标注"
                  >
                    <Trash2 size={12} /> 清空
                  </button>
                </>
              )}
            </div>
          )}
        </div>
      )}

      {/* In-Place Side Analysis Drawer */}
      {sidePanelOpen && analysis.status !== "idle" && panelPos && (
        <aside
          className="overlay-side-panel"
          style={{
            left: panelPos.left,
            top: panelPos.top,
            width: panelW,
            maxHeight: panelMaxH,
          }}
        >
          <div className="result-header" style={{ cursor: "default" }}>
            <div className="mode-pills">
              <button
                type="button"
                className={`mode-pill ${analysis.status === "translate" ? "active" : ""}`}
                onClick={() => switchAnalysisMode("translate")}
              >
                <Languages size={12} /> 翻译
              </button>
              <button
                type="button"
                className={`mode-pill ${analysis.status === "ocr" ? "active" : ""}`}
                onClick={() => switchAnalysisMode("ocr")}
              >
                <ScanText size={12} /> 识字
              </button>
              <button
                type="button"
                className={`mode-pill ${analysis.status === "ask" ? "active" : ""}`}
                onClick={() => switchAnalysisMode("ask")}
              >
                <Sparkles size={12} /> 问 AI
              </button>
            </div>

            <div className="icon-actions">
              <button
                type="button"
                className="icon-btn"
                onClick={runStageToStack}
                title="收入桌面叠堆"
              >
                <Layers size={13} />
              </button>
              <button
                type="button"
                className="icon-btn"
                onClick={runPinAll}
                title="转为桌面贴图"
              >
                <Pin size={13} />
              </button>
              <button
                type="button"
                className="icon-btn"
                onClick={() => invoke("overlay_clear_analysis").catch(console.error)}
                title="收起侧栏"
              >
                <X size={14} />
              </button>
            </div>
          </div>

          {analysis.status === "translate" && (
            <div className="result-subbar">
              <div className="mini-engine-row">
                {ENGINE_OPTIONS.map((opt) => (
                  <button
                    key={opt.id}
                    type="button"
                    className={`mini-chip ${analysis.engine === opt.id ? "active" : ""}`}
                    onClick={() => {
                      setActiveEngine(opt.id);
                      switchAnalysisMode("translate", opt.id, analysis.targetLang);
                    }}
                  >
                    {opt.label}
                  </button>
                ))}
              </div>
              <select
                className="mini-select"
                value={analysis.targetLang}
                onChange={(e) => {
                  const lang = e.target.value;
                  setActiveTargetLang(lang);
                  switchAnalysisMode("translate", analysis.engine, lang);
                }}
              >
                {TARGET_LANGUAGES.map((l) => (
                  <option key={l} value={l}>
                    {l}
                  </option>
                ))}
              </select>
            </div>
          )}

          {analysis.thumbs.length > 1 && (
            <div className="overlay-thumb-row">
              {analysis.thumbs.map((t) => (
                <div key={t.index} className="overlay-thumb-chip">
                  <img src={t.imageDataUrl} alt={t.label} draggable={false} />
                  <span>{t.label}</span>
                </div>
              ))}
            </div>
          )}

          <div className="result-body">
            {(analysis.status === "translate" || analysis.status === "ocr") && (
              <>
                {analysis.loading && (() => {
                  const activeItems =
                    overlayStream &&
                    overlayStream.jobId === analysis.jobId &&
                    overlayStream.kind === analysis.status
                      ? overlayStream.items
                      : null;
                  const streamedIndices = activeItems
                    ? Object.keys(activeItems)
                        .map(Number)
                        .filter((idx) => {
                          const entry = activeItems[idx];
                          return Boolean(entry?.text || entry?.originalText);
                        })
                        .sort((a, b) => a - b)
                    : [];
                  return (
                    <>
                      <div className="loading-status">
                        <Loader2 size={14} className="spin" />
                        <span>
                          {streamedIndices.length > 0
                            ? analysis.status === "ocr"
                              ? "正在流式识别文字..."
                              : `正在流式生成译文 (${analysis.targetLang})...`
                            : analysis.status === "ocr"
                              ? `正在识别 ${analysis.thumbs.length} 个区域文字...`
                              : `正在翻译 ${analysis.thumbs.length} 个区域 (${analysis.targetLang})...`}
                        </span>
                      </div>
                      {streamedIndices.length > 0 && (
                        <div className="batch-item-list">
                          {streamedIndices.map((idx) => {
                            const entry = activeItems![idx];
                            const title = analysis.thumbs[idx]?.label || `区域 ${idx + 1}`;
                            return (
                              <div key={idx} className="batch-item-card">
                                {analysis.thumbs.length > 1 && (
                                  <div className="batch-item-head">
                                    <span>{title}</span>
                                  </div>
                                )}
                                {entry.text && (
                                  <div className="batch-item-text">
                                    {entry.text}
                                    <span className="stream-cursor" />
                                  </div>
                                )}
                                {analysis.status === "translate" && entry.originalText && (
                                  <div className="batch-item-orig">
                                    {entry.originalText}
                                    {!entry.text && <span className="stream-cursor" />}
                                  </div>
                                )}
                              </div>
                            );
                          })}
                        </div>
                      )}
                    </>
                  );
                })()}

                {analysis.error && <div className="error-box">{analysis.error}</div>}

                {!analysisMatchesRegions && !analysis.loading && (
                  <div className="error-box">选区已修改，请重新选择翻译或识字。</div>
                )}

                {analysisMatchesRegions && !analysis.loading && analysis.items.length > 0 && (
                  <>
                    <div className="result-section-head">
                      <span>
                        {analysis.items.length} 个区域 · {analysis.model} · {analysis.elapsedMs}ms
                      </span>
                      <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                        {analysis.status === "translate" && (
                          <button
                            type="button"
                            className={`mini-chip ${inPlacePreview ? "active" : ""}`}
                            onClick={() => setInPlacePreview((v) => !v)}
                            title="直接在选区原位擦除原文并覆盖显示译文"
                          >
                            {inPlacePreview ? "关闭原位" : "原位预览"}
                          </button>
                        )}
                        <button
                          type="button"
                          className="btn btn-sm"
                          onClick={() =>
                            copyText(
                              analysis.items
                                .map((it) =>
                                  analysis.items.length > 1
                                    ? `[${it.title}]\n${
                                        analysis.status === "ocr"
                                          ? it.originalText || it.translatedText
                                          : it.translatedText
                                      }`
                                    : analysis.status === "ocr"
                                      ? it.originalText || it.translatedText
                                      : it.translatedText
                                )
                                .join("\n\n"),
                              "all"
                            )
                          }
                        >
                          {copiedKey === "all" ? <Check size={12} /> : <Copy size={12} />}
                          复制全部
                        </button>
                      </div>
                    </div>

                    {analysis.status === "translate" && (
                      <div className="inplace-export-bar">
                        <button
                          type="button"
                          className="btn btn-sm"
                          onClick={() => runExportTranslated("copy")}
                          title="将原位译文覆盖截图复制到剪贴板"
                        >
                          <Copy size={12} /> 复制译文图
                        </button>
                        <button
                          type="button"
                          className="btn btn-sm"
                          onClick={() => runExportTranslated("save")}
                          title="保存原位译文截图 PNG"
                        >
                          <Download size={12} /> 保存译文图
                        </button>
                        <button
                          type="button"
                          className="btn btn-sm"
                          onClick={() => runExportTranslated("pin")}
                          title="将原位译文截图贴到桌面置顶"
                        >
                          <Pin size={12} /> 贴图译文
                        </button>
                      </div>
                    )}

                    <div className="batch-item-list">
                      {analysis.items.map((it, itemIdx) => {
                        const mainText =
                          analysis.status === "ocr"
                            ? it.originalText || it.translatedText
                            : it.translatedText;
                        const alignedBlocks =
                          analysis.status === "translate" && it.blocks && it.blocks.length > 1
                            ? it.blocks
                            : null;
                        return (
                          <div key={it.index} className="batch-item-card">
                            <div className="batch-item-head">
                              <span>
                                {it.title} · {it.sourceLang}
                              </span>
                              <button
                                type="button"
                                className="icon-btn"
                                onClick={() => copyText(mainText, `item-${it.index}`)}
                                title="复制"
                              >
                                {copiedKey === `item-${it.index}` ? (
                                  <Check size={12} />
                                ) : (
                                  <Copy size={12} />
                                )}
                              </button>
                            </div>
                            {alignedBlocks ? (
                              <div className="bilingual-pair-list">
                                {alignedBlocks.map((blk, bIdx) => (
                                  <div
                                    key={bIdx}
                                    className={`bilingual-pair-row ${hoveredBlockKey === `${itemIdx}-${bIdx}` ? "active" : ""}`}
                                    onMouseEnter={() => setHoveredBlockKey(`${itemIdx}-${bIdx}`)}
                                    onMouseLeave={() =>
                                      setHoveredBlockKey((cur) =>
                                        cur === `${itemIdx}-${bIdx}` ? null : cur
                                      )
                                    }
                                  >
                                    <span className="bilingual-pair-idx">{bIdx + 1}</span>
                                    <div className="bilingual-pair-body">
                                      <div className="bilingual-pair-trans">
                                        {blk.translatedText}
                                      </div>
                                      {blk.originalText && (
                                        <div className="bilingual-pair-orig">
                                          {blk.originalText}
                                        </div>
                                      )}
                                    </div>
                                  </div>
                                ))}
                              </div>
                            ) : (
                              <>
                                <div className="batch-item-text">
                                  {mainText || "（未检测到文字）"}
                                </div>
                                {analysis.status === "translate" && it.originalText && (
                                  <div className="batch-item-orig">{it.originalText}</div>
                                )}
                              </>
                            )}
                          </div>
                        );
                      })}
                    </div>
                  </>
                )}
              </>
            )}

            {analysis.status === "ask" && (
              <>
                <div className="ask-thread">
                  {analysis.messages.map((msg, i) => (
                    <div key={i} className={`ask-bubble ${msg.role}`}>
                      {msg.content}
                    </div>
                  ))}
                  {analysis.loading &&
                    (overlayStream &&
                    overlayStream.jobId === analysis.jobId &&
                    overlayStream.kind === "ask" &&
                    overlayStream.text ? (
                      <div className="ask-bubble assistant streaming">
                        {overlayStream.text}
                        <span className="stream-cursor" />
                      </div>
                    ) : (
                      <div className="loading-status">
                        <Loader2 size={13} className="spin" />
                        <span>AI 正在结合选中区域思考...</span>
                      </div>
                    ))}
                  {analysis.error && <div className="error-box">{analysis.error}</div>}
                  <div ref={askThreadEndRef} />
                </div>
              </>
            )}
          </div>

          {analysis.status === "ask" && (
            <>
              <AskQuickActionsBar
                loading={analysis.loading}
                onTrigger={(prompt) => void sendOverlayAsk(prompt)}
                compact
              />
              <div className="ask-input-dock">
                <textarea
                  className="ask-textarea"
                  rows={1}
                  value={askInput}
                  onChange={(e) => setAskInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                      e.preventDefault();
                      sendOverlayAsk();
                    }
                  }}
                  placeholder={`针对这 ${analysis.thumbs.length} 个区域向 AI 提问...`}
                />
                <button
                  type="button"
                  className="btn btn-primary btn-sm"
                  onClick={() => sendOverlayAsk()}
                  disabled={analysis.loading || !askInput.trim()}
                >
                  <Send size={13} />
                </button>
              </div>
            </>
          )}
        </aside>
      )}

      {stagedStatus && stackFlying.length === 0 && (
        <div
          className={`overlay-staged-dock ${draftRect ? "dragging" : ""}`}
          onMouseDown={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.stopPropagation()}
        >
          {stagedStatus.latestThumb ? (
            <img
              src={stagedStatus.latestThumb}
              alt="最新入叠堆截图"
              className="overlay-staged-thumb"
              draggable={false}
            />
          ) : (
            <div className="overlay-staged-thumb overlay-staged-thumb-icon">
              <Layers size={16} />
            </div>
          )}
          <div className="overlay-staged-meta">
            <div className="overlay-staged-title">
              已入叠堆 {stagedStatus.count}/{stagedStatus.maxSize}
            </div>
            <div className="overlay-staged-sub">
              {stagedStatus.count >= stagedStatus.maxSize
                ? "叠堆已满 · 点击完成查看"
                : "继续框选下一张 · Esc 完成"}
            </div>
          </div>
          <button
            type="button"
            className="overlay-staged-done-btn"
            onClick={() => void runFinishStackStaging()}
            disabled={busy}
            title="结束连续框选并打开叠堆 (Esc)"
          >
            完成
          </button>
        </div>
      )}

      {stackFlying.length > 0 && (
        <div className="overlay-stack-fly-layer">
          <div className="overlay-stack-drop-target">
            <Layers size={18} />
            <span>放入右下角叠堆</span>
          </div>
          {stackFlying.map((card) => (
            <div
              key={card.id}
              className="overlay-stack-fly-card"
              ref={(el) => {
                if (!el || el.dataset.animated) return;
                el.dataset.animated = "1";
                const arcX = card.dx * 0.42;
                const arcY = Math.min(card.dy * 0.3, -48);
                const motion = el.animate(
                  [
                    {
                      transform: "translate3d(0px, 0px, 0) scale(1) rotate(0deg)",
                      opacity: 1,
                    },
                    {
                      transform: `translate3d(${arcX}px, ${arcY}px, 0) scale(${(1 + card.sx) * 0.52}, ${(1 + card.sy) * 0.52}) rotate(${card.rot * 0.65}deg)`,
                      opacity: 0.96,
                      offset: 0.44,
                    },
                    {
                      transform: `translate3d(${card.dx}px, ${card.dy}px, 0) scale(${card.sx}, ${card.sy}) rotate(${card.rot}deg)`,
                      opacity: card.onLand ? 1 : 0.25,
                    },
                  ],
                  {
                    duration: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? 1 : 390,
                    delay: card.delayMs,
                    easing: "cubic-bezier(0.22, 0.85, 0.36, 1)",
                    fill: "both",
                  }
                );
                if (card.onLand) void motion.finished.then(card.onLand, card.onLand);
              }}
              style={{
                left: card.x,
                top: card.y,
                width: card.w,
                height: card.h,
              }}
            >
              <canvas ref={(canvas) => paintFlightCard(canvas, card)} aria-hidden="true" />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
