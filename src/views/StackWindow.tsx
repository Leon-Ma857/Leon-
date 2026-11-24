import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { listenToWindow as listen } from "../utils/windowEvents";
import { ALT_LABEL, MOD_LABEL } from "../utils/platform";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  ArrowUpRight,
  Camera,
  Check,
  ChevronDown,
  ChevronUp,
  Circle,
  Columns2,
  Copy,
  Crop,
  Download,
  ExternalLink,
  Eye,
  Files,
  FileText,
  Grid,
  GripVertical,
  Hand,
  Languages,
  Layers,
  ListOrdered,
  Loader2,
  Minimize2,
  Pencil,
  Pin,
  QrCode,
  RotateCcw,
  Rows2,
  ScanText,
  Send,
  Sparkles,
  Square,
  Trash2,
  Type,
  Undo2,
  Wand2,
  X,
} from "lucide-react";
import {
  ENGINE_OPTIONS,
  StackItemImageDto,
  StackViewState,
  StitchDirection,
  StreamPreviewEvent,
  TARGET_LANGUAGES,
  TranslateEngine,
} from "../types";
import { AskQuickActionsBar } from "../components/AskQuickActionsBar";
import { Popconfirm } from "../components/Popconfirm";
import { SlidingSegmented } from "../components/SlidingSegmented";

const COLLAPSED_SIZE = { width: 340, height: 244 };
const PREVIEW_SIZE = { width: 820, height: 560 };
const EXPANDED_SIZE = { width: 472, height: 552 };
const STACK_SIZES = { collapsed: COLLAPSED_SIZE, preview: PREVIEW_SIZE, workspace: EXPANDED_SIZE };
const STACK_MOTION = { duration: 285, easing: "cubic-bezier(0.23, 0.65, 0.32, 1)" };
const DELETE_STYLE_STORAGE_KEY = "luma.stackDeleteStyle";

type StackWorkspaceTab = "cards" | "stitch" | "translate" | "ocr" | "ask";
type StackDeleteStyle = "peel" | "blocks";

function readStoredDeleteStyle(): StackDeleteStyle {
  try {
    return window.localStorage.getItem(DELETE_STYLE_STORAGE_KEY) === "blocks" ? "blocks" : "peel";
  } catch {
    return "peel";
  }
}

function waitForStackViewport({ width, height }: typeof COLLAPSED_SIZE) {
  // At fractional DPI a rounded native dimension can be one CSS pixel smaller.
  const fits = () => window.innerWidth >= width - 1 && window.innerHeight >= height - 1;
  if (fits()) return Promise.resolve();

  return new Promise<void>((resolve, reject) => {
    const onResize = () => {
      if (!fits()) return;
      window.clearTimeout(timeout);
      window.removeEventListener("resize", onResize);
      resolve();
    };
    const timeout = window.setTimeout(() => {
      window.removeEventListener("resize", onResize);
      reject(new Error("叠堆视图调整超时"));
    }, 500);
    window.addEventListener("resize", onResize);
    onResize();
  });
}

type StackViewMode = "collapsed" | "preview" | "workspace";
type PreviewTool = "pan" | "crop" | "rect" | "ellipse" | "arrow" | "pen" | "mosaic" | "text";

interface Point {
  x: number;
  y: number;
}

interface ImageRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

type StackAnnotation =
  | { kind: "rect" | "ellipse" | "arrow"; color: string; width: number; x1: number; y1: number; x2: number; y2: number }
  | { kind: "pen"; color: string; width: number; points: Point[] }
  | { kind: "mosaic"; blockSize: number; points: Point[] }
  | { kind: "text"; color: string; fontSize: number; x: number; y: number; text: string };

interface EditSnapshot {
  dataUrl: string;
  width: number;
  height: number;
  annotations: StackAnnotation[];
}

const PREVIEW_COLORS = ["#ef4444", "#f59e0b", "#0a84ff", "#22c55e", "#ffffff", "#111111"];
const STROKE_WIDTHS = [2, 4, 7];

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
  item: StackAnnotation,
  mosaicSampleCtx: CanvasRenderingContext2D | null
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
    ctx.font = `600 ${item.fontSize}px -apple-system, "SF Pro Text", "Segoe UI", "PingFang SC", sans-serif`;
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

          let fill = "rgba(38, 38, 38, 0.88)";
          if (mosaicSampleCtx) {
            const sx = Math.max(0, Math.min(mosaicSampleCtx.canvas.width - 1, Math.round(gx + b / 2)));
            const sy = Math.max(0, Math.min(mosaicSampleCtx.canvas.height - 1, Math.round(gy + b / 2)));
            const p = mosaicSampleCtx.getImageData(sx, sy, 1, 1).data;
            fill = `rgba(${p[0]}, ${p[1]}, ${p[2]}, 0.96)`;
          }
          ctx.fillStyle = fill;
          ctx.fillRect(gx, gy, b + 0.5, b + 0.5);
        }
      }
    }
  }
}

export function StackWindow() {
  const [state, setState] = useState<StackViewState | null>(null);
  const [askStream, setAskStream] = useState<{ jobId: number; text: string } | null>(null);
  const [batchStream, setBatchStream] = useState<{
    jobId: number;
    kind: string;
    items: Record<number, { text: string; originalText?: string }>;
  } | null>(null);
  const [viewMode, setViewMode] = useState<StackViewMode>("collapsed");
  const [leavingMode, setLeavingMode] = useState<StackViewMode | null>(null);
  const [activeTab, setActiveTab] = useState<StackWorkspaceTab>("cards");
  const [deleteStyle, setDeleteStyle] = useState<StackDeleteStyle>(readStoredDeleteStyle);
  const [direction, setDirection] = useState<StitchDirection>("vertical");
  const [engineChoice, setEngine] = useState<TranslateEngine | null>(null);
  const [targetLangChoice, setTargetLang] = useState<string | null>(null);
  const engine = engineChoice ?? state?.engine ?? "ai";
  const targetLang = targetLangChoice ?? state?.targetLang ?? "简体中文";
  const [askInput, setAskInput] = useState("");
  const [feedback, setFeedback] = useState<string | null>(null);
  const [undoToast, setUndoToast] = useState<{ title: string; indexLabel: string } | null>(null);
  const [undoRestoring, setUndoRestoring] = useState(false);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [qrResults, setQrResults] = useState<string[] | null>(null);
  const [dragReorder, setDragReorder] = useState<{
    fromIdx: number;
    dropSlot: number;
    dx: number;
    dy: number;
    active: boolean;
  } | null>(null);
  const [verticalDrag, setVerticalDrag] = useState<{
    fromIdx: number;
    dropSlot: number;
    dy: number;
    lineTop: number;
    active: boolean;
  } | null>(null);
  const thumbStripRef = useRef<HTMLDivElement | null>(null);
  const dragListRef = useRef<HTMLDivElement | null>(null);
  const suppressEdgeClickRef = useRef(false);

  // Corner stack catch-bounce & slot-digit roll state
  const [catchBounce, setCatchBounce] = useState(false);
  const [countRollKey, setCountRollKey] = useState(0);
  const prevCountRef = useRef<number | null>(null);
  const catchBounceTimerRef = useRef<number | null>(null);

  // Direct card-stack preview & exit/restore animation state
  const [activeIndex, setActiveIndex] = useState(0);
  const [fullImages, setFullImages] = useState<Record<number, StackItemImageDto>>({});
  const [slide, setSlide] = useState<1 | -1 | null>(null);
  const [removing, setRemoving] = useState(false);
  const [removingThumbIds, setRemovingThumbIds] = useState<Record<number, StackDeleteStyle>>({});
  const [nudgeThumbIds, setNudgeThumbIds] = useState<Record<number, true>>({});
  const [justDroppedId, setJustDroppedId] = useState<number | null>(null);
  const [justRestoredId, setJustRestoredId] = useState<number | null>(null);
  const dropSettleTimerRef = useRef<number | null>(null);
  const nudgeTimerRef = useRef<number | null>(null);
  const restoreTimerRef = useRef<number | null>(null);
  const undoToastTimerRef = useRef<number | null>(null);
  const undoToastHoveredRef = useRef(false);
  const stateRef = useRef<StackViewState | null>(null);
  stateRef.current = state;
  const [tool, setTool] = useState<PreviewTool>("pan");
  const [color, setColor] = useState(PREVIEW_COLORS[0]);
  const [strokeWidth, setStrokeWidth] = useState(4);
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState<Point>({ x: 0, y: 0 });
  const [editorImage, setWorkingImage] = useState<(Omit<EditSnapshot, "annotations"> & { id: number }) | null>(null);
  const [annotations, setAnnotations] = useState<StackAnnotation[]>([]);
  const [draftAnnot, setDraftAnnot] = useState<StackAnnotation | null>(null);
  const [cropRect, setCropRect] = useState<ImageRect | null>(null);
  const [undoStack, setUndoStack] = useState<EditSnapshot[]>([]);
  const [hasUnsavedEdits, setHasUnsavedEdits] = useState(false);
  const [savingEdits, setSavingEdits] = useState(false);
  const [textDraft, setTextDraft] = useState<{ x: number; y: number; value: string } | null>(null);
  const [vpSize, setVpSize] = useState({ w: PREVIEW_SIZE.width - 40, h: PREVIEW_SIZE.height - 24 });

  const threadEndRef = useRef<HTMLDivElement | null>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const canvasWrapRef = useRef<HTMLDivElement | null>(null);
  const annotCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const sampleCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const textInputRef = useRef<HTMLTextAreaElement | null>(null);
  const backdropPressRef = useRef<{ x: number; y: number } | null>(null);
  const suppressBlurCollapseRef = useRef(false);
  const switchViewModeRef = useRef<(nextMode: StackViewMode, initialIndex?: number, targetTab?: StackWorkspaceTab) => Promise<void>>(async () => {});
  const saveInFlight = useRef<Promise<boolean> | null>(null);
  const switchingView = useRef(false);
  const stackGeneration = useRef(0);
  const modeLayers = useRef<Partial<Record<StackViewMode, HTMLDivElement | null>>>({});
  const loadingImages = useRef(new Set<number>());
  const askSubmitting = useRef(false);
  const dragRef = useRef<
    | { kind: "none" }
    | { kind: "pan"; sx: number; sy: number; orig: Point }
    | { kind: "crop"; sx: number; sy: number }
    | { kind: "draw" }
  >({ kind: "none" });

  useEffect(() => {
    return () => {
      if (dropSettleTimerRef.current !== null) {
        window.clearTimeout(dropSettleTimerRef.current);
      }
      if (nudgeTimerRef.current !== null) {
        window.clearTimeout(nudgeTimerRef.current);
      }
      if (restoreTimerRef.current !== null) {
        window.clearTimeout(restoreTimerRef.current);
      }
      if (catchBounceTimerRef.current !== null) {
        window.clearTimeout(catchBounceTimerRef.current);
      }
      if (undoToastTimerRef.current !== null) {
        window.clearTimeout(undoToastTimerRef.current);
      }
    };
  }, []);

  // Trigger catch-bounce & slot-digit roll whenever the stack count changes.
  useEffect(() => {
    const count = state?.items.length ?? 0;
    if (prevCountRef.current !== null && count !== prevCountRef.current) {
      setCountRollKey((k) => k + 1);
      if (count > prevCountRef.current) {
        setCatchBounce(false);
        requestAnimationFrame(() => {
          setCatchBounce(true);
        });
        if (catchBounceTimerRef.current !== null) {
          window.clearTimeout(catchBounceTimerRef.current);
        }
        catchBounceTimerRef.current = window.setTimeout(() => {
          catchBounceTimerRef.current = null;
          setCatchBounce(false);
        }, 420);
      }
    }
    prevCountRef.current = count;
  }, [state?.items.length]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let unlistenStream: (() => void) | undefined;

    invoke<StackViewState>("get_stack_state")
      .then((initial) => {
        setState(initial);
      })
      .catch(console.error);

    listen<StackViewState>("stack:updated", (event) => {
      const next = event.payload;
      setState(next);
      if (!next.askLoading) {
        setAskStream(null);
      } else {
        setAskStream((cur) => (cur && cur.jobId === next.askJobId ? cur : null));
      }
      if (!next.batchLoading) {
        setBatchStream(null);
      } else {
        setBatchStream((cur) => (cur && cur.jobId === next.batchJobId ? cur : null));
      }
      if (next.items.length === 0) {
        stackGeneration.current += 1;
        setLeavingMode(null);
        setEngine(null);
        setTargetLang(null);
        setViewMode("collapsed");
        setFullImages({});
        setRemovingThumbIds({});
        setVerticalDrag(null);
        setDragReorder(null);
        setWorkingImage(null);
        setAnnotations([]);
        setUndoStack([]);
        setHasUnsavedEdits(false);
        if (!next.canUndoRemove) {
          setUndoToast(null);
        }
        invoke("resize_stack_window", COLLAPSED_SIZE).catch(console.error);
      } else {
        setActiveIndex((cur) => Math.max(0, Math.min(cur, next.items.length - 1)));
      }
    }).then((fn) => {
      unlisten = fn;
    });

    listen<StreamPreviewEvent>("stack:stream", (event) => {
      const p = event.payload;
      if (p.kind === "ask") {
        setAskStream({ jobId: p.jobId, text: p.text });
      } else if (p.itemIndex != null) {
        setBatchStream((prev) => {
          const base =
            prev && prev.jobId === p.jobId && prev.kind === p.kind
              ? prev
              : { jobId: p.jobId, kind: p.kind, items: {} };
          return {
            ...base,
            items: {
              ...base.items,
              [p.itemIndex!]: {
                text: p.text,
                originalText: p.originalText ?? undefined,
              },
            },
          };
        });
      }
    }).then((fn) => {
      unlistenStream = fn;
    });

    return () => {
      if (unlisten) unlisten();
      if (unlistenStream) unlistenStream();
    };
  }, []);

  const landingCaptureId = state?.landingCaptureId;
  useEffect(() => {
    if (!landingCaptureId) return;
    let frame = 0;
    let cancelled = false;
    const images = Array.from(document.querySelectorAll<HTMLImageElement>(
      ".corner-stack img, .stack-preview-paper img, .stack-workspace img",
    ));
    void Promise.allSettled(images.map((image) => image.decode())).then(() => {
      if (cancelled) return;
      frame = requestAnimationFrame(() => {
        frame = requestAnimationFrame(() => {
          void invoke("stack_frame_ready", { captureId: landingCaptureId }).catch(console.error);
        });
      });
    });
    return () => {
      cancelled = true;
      cancelAnimationFrame(frame);
    };
  }, [landingCaptureId]);

  useEffect(() => {
    if (viewMode === "workspace" && activeTab === "ask") {
      threadEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [viewMode, activeTab, state?.askMessages, askStream]);

  useEffect(() => {
    const el = viewportRef.current;
    if (!el || viewMode !== "preview") return;
    const updateSize = () => {
      const w = el.clientWidth;
      const h = el.clientHeight;
      if (w > 40 && h > 40) {
        setVpSize((size) => size.w === w && size.h === h ? size : { w, h });
      }
    };
    updateSize();
    const ro = new ResizeObserver(updateSize);
    ro.observe(el);
    return () => ro.disconnect();
  }, [viewMode]);

  useEffect(() => {
    if (textDraft) {
      setTimeout(() => textInputRef.current?.focus(), 0);
    }
  }, [textDraft]);

  const showNotice = useCallback((text: string) => {
    setFeedback(text);
    setTimeout(() => setFeedback((cur) => (cur === text ? null : cur)), 2200);
  }, []);

  const clearUndoTimer = useCallback(() => {
    if (undoToastTimerRef.current !== null) {
      window.clearTimeout(undoToastTimerRef.current);
      undoToastTimerRef.current = null;
    }
  }, []);

  const scheduleUndoDismiss = useCallback(() => {
    clearUndoTimer();
    undoToastTimerRef.current = window.setTimeout(() => {
      undoToastTimerRef.current = null;
      if (undoToastHoveredRef.current) return;
      setUndoToast(null);
      if ((stateRef.current?.items.length ?? 0) === 0) {
        invoke("dismiss_stack_undo").catch(console.error);
      }
    }, 4600);
  }, [clearUndoTimer]);

  const showUndoToast = useCallback(
    (title: string, indexLabel: string) => {
      setUndoToast({ title, indexLabel });
      scheduleUndoDismiss();
    },
    [scheduleUndoDismiss]
  );

  const handleDismissUndoToast = useCallback(() => {
    clearUndoTimer();
    setUndoToast(null);
    if ((stateRef.current?.items.length ?? 0) === 0) {
      invoke("dismiss_stack_undo").catch(console.error);
    }
  }, [clearUndoTimer]);

  const handleUndoRemove = useCallback(async () => {
    if (undoRestoring) return;
    clearUndoTimer();
    setUndoRestoring(true);
    try {
      const restoredId = await invoke<number | null>("restore_last_removed_stack_item");
      setUndoToast(null);
      if (restoredId !== null) {
        setJustRestoredId(restoredId);
        if (restoreTimerRef.current !== null) {
          window.clearTimeout(restoreTimerRef.current);
        }
        restoreTimerRef.current = window.setTimeout(() => {
          restoreTimerRef.current = null;
          setJustRestoredId(null);
        }, 420);
        showNotice("已恢复该叠堆卡片");
      }
    } catch (err) {
      showNotice(String(err));
    } finally {
      setUndoRestoring(false);
    }
  }, [clearUndoTimer, showNotice, undoRestoring]);

  const toggleDeleteStyle = useCallback(() => {
    setDeleteStyle((prev) => {
      const next: StackDeleteStyle = prev === "peel" ? "blocks" : "peel";
      try {
        window.localStorage.setItem(DELETE_STYLE_STORAGE_KEY, next);
      } catch {
        // ignore storage restrictions
      }
      return next;
    });
  }, []);

  useEffect(() => {
    if (viewMode === "preview") return;
    const onKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      const inEditable = Boolean(
        target &&
          (target.tagName === "INPUT" ||
            target.tagName === "TEXTAREA" ||
            target.isContentEditable)
      );
      if (inEditable) return;
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.key.toLowerCase() === "z") {
        if (undoToast || stateRef.current?.canUndoRemove) {
          e.preventDefault();
          void handleUndoRemove();
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [viewMode, undoToast, handleUndoRemove]);

  const markCopied = useCallback((key: string) => {
    setCopiedKey(key);
    setTimeout(() => setCopiedKey((cur) => (cur === key ? null : cur)), 1500);
  }, []);

  const activeItem = useMemo(() => {
    if (!state || state.items.length === 0) return null;
    return state.items[Math.max(0, Math.min(activeIndex, state.items.length - 1))] ?? null;
  }, [state, activeIndex]);

  const workingImage = editorImage && editorImage.id === activeItem?.id ? editorImage : null;

  // Use the existing thumbnail during the transition and decode the sharp image before swapping it in.
  useEffect(() => {
    if (viewMode !== "preview" || leavingMode || !state) return;
    const generation = stackGeneration.current;
    for (const { id } of state.items) {
      if (fullImages[id] || loadingImages.current.has(id)) continue;
      loadingImages.current.add(id);
      void invoke<StackItemImageDto>("get_stack_item_image", { id })
        .then(async (dto) => {
          const image = new Image();
          image.src = dto.imageDataUrl;
          await image.decode();
          if (generation === stackGeneration.current) {
            setFullImages((prev) => ({ ...prev, [dto.id]: dto }));
          }
        })
        .catch(console.error)
        .finally(() => loadingImages.current.delete(id));
    }
  }, [viewMode, leavingMode, state, fullImages]);

  // Sync working image when active card changes or full-res image arrives
  useEffect(() => {
    if (viewMode !== "preview" || !activeItem) return;
    const dto = fullImages[activeItem.id];
    if (!dto) return;
    const nextUrl = dto.imageDataUrl;
    const nextW = dto.pixelWidth;
    const nextH = dto.pixelHeight;
    setWorkingImage((prev) => {
      if (prev?.id === dto.id && (hasUnsavedEdits || prev.dataUrl === nextUrl)) return prev;
      return { id: dto.id, dataUrl: nextUrl, width: nextW, height: nextH };
    });
  }, [viewMode, activeItem, fullImages, hasUnsavedEdits]);

  // Update offscreen sample canvas for mosaic tool
  useEffect(() => {
    if (!workingImage) {
      sampleCanvasRef.current = null;
      return;
    }
    const img = new Image();
    img.onload = () => {
      const c = document.createElement("canvas");
      c.width = img.naturalWidth || workingImage.width;
      c.height = img.naturalHeight || workingImage.height;
      const sctx = c.getContext("2d", { willReadFrequently: true });
      if (sctx) {
        sctx.drawImage(img, 0, 0);
        sampleCanvasRef.current = c;
      }
    };
    img.src = workingImage.dataUrl;
  }, [workingImage]);

  const imgW = Math.max(workingImage?.width ?? activeItem?.pixelWidth ?? 1, 1);
  const imgH = Math.max(workingImage?.height ?? activeItem?.pixelHeight ?? 1, 1);
  const showAnnotSubbar =
    tool === "rect" ||
    tool === "ellipse" ||
    tool === "arrow" ||
    tool === "pen" ||
    tool === "mosaic" ||
    tool === "text";
  const stageAvailW = Math.max(240, vpSize.w - 156);
  const stageAvailH = Math.max(140, vpSize.h - (showAnnotSubbar ? 172 : 126));
  const fitScale = useMemo(() => {
    return Math.min(stageAvailW / imgW, stageAvailH / imgH, 1.45);
  }, [stageAvailW, stageAvailH, imgW, imgH]);

  const baseDispW = Math.max(40, Math.round(imgW * fitScale));
  const baseDispH = Math.max(40, Math.round(imgH * fitScale));
  const dispW = Math.max(40, Math.round(baseDispW * zoom));
  const dispH = Math.max(40, Math.round(baseDispH * zoom));
  const paperBoxW = Math.min(dispW, stageAvailW);
  const paperBoxH = Math.min(dispH, stageAvailH);
  const pxScale = dispW / imgW;

  // Repaint annotation canvas in image-pixel space scaled cleanly to display size
  useEffect(() => {
    const canvas = annotCanvasRef.current;
    if (!canvas || viewMode !== "preview") return;
    const dpr = window.devicePixelRatio || 1;
    const targetW = Math.max(1, Math.round(dispW * dpr));
    const targetH = Math.max(1, Math.round(dispH * dpr));
    if (canvas.width !== targetW || canvas.height !== targetH) {
      canvas.width = targetW;
      canvas.height = targetH;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr * pxScale, 0, 0, dpr * pxScale, 0, 0);
    ctx.clearRect(0, 0, imgW, imgH);
    const sampleCtx = sampleCanvasRef.current?.getContext("2d") ?? null;
    for (const item of annotations) {
      renderAnnotationItem(ctx, item, sampleCtx);
    }
    if (draftAnnot) {
      renderAnnotationItem(ctx, draftAnnot, sampleCtx);
    }
  }, [annotations, draftAnnot, dispW, dispH, imgW, imgH, pxScale, viewMode]);

  const flattenWorkingToDataUrl = useCallback(async (): Promise<string | null> => {
    if (!workingImage) return null;
    const imgEl = await new Promise<HTMLImageElement>((resolve, reject) => {
      const im = new Image();
      im.onload = () => resolve(im);
      im.onerror = reject;
      im.src = workingImage.dataUrl;
    });
    const out = document.createElement("canvas");
    out.width = imgEl.naturalWidth || workingImage.width;
    out.height = imgEl.naturalHeight || workingImage.height;
    const ctx = out.getContext("2d");
    if (!ctx) return null;
    ctx.drawImage(imgEl, 0, 0, out.width, out.height);
    const sampleCtx = sampleCanvasRef.current?.getContext("2d") ?? null;
    for (const item of annotations) {
      renderAnnotationItem(ctx, item, sampleCtx);
    }
    return out.toDataURL("image/png");
  }, [workingImage, annotations]);

  const persistCurrentEdits = useCallback(async () => {
    if (!activeItem || !hasUnsavedEdits) return true;
    setSavingEdits(true);
    try {
      const dataUrl = await flattenWorkingToDataUrl();
      if (!dataUrl) return false;
      const updated = await invoke<StackItemImageDto>("update_stack_item_image", {
        id: activeItem.id,
        imageBase64: dataUrl,
      });
      setFullImages((prev) => ({ ...prev, [updated.id]: updated }));
      setWorkingImage({
        id: updated.id,
        dataUrl: updated.imageDataUrl,
        width: updated.pixelWidth,
        height: updated.pixelHeight,
      });
      setAnnotations([]);
      setHasUnsavedEdits(false);
      showNotice("已保存到叠堆");
      return true;
    } catch (err) {
      showNotice(String(err));
      return false;
    } finally {
      setSavingEdits(false);
    }
  }, [activeItem, flattenWorkingToDataUrl, hasUnsavedEdits, showNotice]);

  const saveCurrentEdits = useCallback(() => {
    if (saveInFlight.current) return saveInFlight.current;
    const save = persistCurrentEdits().finally(() => {
      saveInFlight.current = null;
    });
    saveInFlight.current = save;
    return save;
  }, [persistCurrentEdits]);

  const resetPreviewEditorState = useCallback(() => {
    setTool("pan");
    setZoom(1);
    setPan({ x: 0, y: 0 });
    setAnnotations([]);
    setDraftAnnot(null);
    setCropRect(null);
    setUndoStack([]);
    setHasUnsavedEdits(false);
    setTextDraft(null);
  }, []);

  const switchViewMode = useCallback(
    async (
      nextMode: StackViewMode,
      initialIndex?: number,
      targetTab?: StackWorkspaceTab
    ) => {
      if (switchingView.current || saveInFlight.current || removing) return;
      switchingView.current = true;
      const generation = stackGeneration.current;
      try {
        if (viewMode === "preview" && nextMode !== "preview" && hasUnsavedEdits) {
          if (!(await saveCurrentEdits())) return;
        }
        if (generation !== stackGeneration.current) return;

        const from = STACK_SIZES[viewMode];
        const to = STACK_SIZES[nextMode];
        const reserve = { width: Math.max(from.width, to.width), height: Math.max(from.height, to.height) };
        if (reserve.width !== from.width || reserve.height !== from.height) {
          // Reveal enough of the fixed Windows surface before drawing the new view.
          await invoke("resize_stack_window", reserve);
          if (generation !== stackGeneration.current) return;
          await waitForStackViewport(reserve);
        }
        if (generation !== stackGeneration.current) return;

        const animate = nextMode !== viewMode;
        const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
        if (nextMode === "preview") {
          void getCurrentWebviewWindow().setFocus().catch(() => {});
        }
        flushSync(() => {
          if (nextMode === "preview") {
            setSlide(null);
            resetPreviewEditorState();
            if (typeof initialIndex === "number") {
              setActiveIndex(initialIndex);
            } else if (state && state.items.length > 0) {
              setActiveIndex(state.items.length - 1);
            }
            setWorkingImage(null);
          }
          if (nextMode === "workspace" && targetTab) {
            setActiveTab(targetTab);
          }
          setLeavingMode(animate ? viewMode : null);
          setViewMode(nextMode);
        });
        if (nextMode === "workspace" && (targetTab === "translate" || targetTab === "ocr")) {
          const hasCurrentBatch = state?.batchMode === targetTab &&
            (state.batchLoading || state.batchItems.length > 0);
          if (!hasCurrentBatch) {
            invoke("stack_run_batch", {
              mode: targetTab,
              engineOverride: engine,
              targetLangOverride: targetLang,
            }).catch((err) => showNotice(String(err)));
          }
        }

        if (animate) {
          const outgoing = modeLayers.current[viewMode];
          const incoming = modeLayers.current[nextMode];
          const isDrawerToggle =
            (viewMode === "collapsed" && nextMode === "workspace") ||
            (viewMode === "workspace" && nextMode === "collapsed");
          const motion = reducedMotion
            ? { duration: 120, easing: "ease-out" }
            : isDrawerToggle
              ? { duration: 240, easing: "cubic-bezier(0.22, 1, 0.36, 1)" }
              : STACK_MOTION;
          if (isDrawerToggle) {
            // Keep the bottom-right corner stack rock-steady while the floating drawer pops out / tucks in.
            const opening = nextMode === "workspace";
            const drawerEl = (opening ? incoming : outgoing)?.querySelector<HTMLElement>(".stack-workspace");
            const redundantAnchor = outgoing?.querySelector<HTMLElement>(".stack-corner-anchor");
            if (redundantAnchor) {
              redundantAnchor.style.opacity = "0";
            }
            const drawerAnim = drawerEl?.animate(
              opening
                ? [
                    { opacity: 0, transform: reducedMotion ? "none" : "translate(14px, 22px) scale(0.91)" },
                    { opacity: 1, transform: "none" },
                  ]
                : [
                    { opacity: 1, transform: "none" },
                    { opacity: 0, transform: reducedMotion ? "none" : "translate(14px, 22px) scale(0.91)" },
                  ],
              { ...motion, fill: "forwards" }
            );
            await Promise.allSettled([drawerAnim?.finished]);
            if (generation !== stackGeneration.current) return;
            flushSync(() => setLeavingMode(null));
            drawerAnim?.cancel();
            if (redundantAnchor) {
              redundantAnchor.style.opacity = "";
            }
          } else {
            const scale = (mode: StackViewMode) => reducedMotion ? "none" : mode === "collapsed" ? "scale(0.92)" : "scale(0.74)";
            const exit = outgoing?.animate([
              { opacity: 1, transform: "none" },
              { opacity: 1, offset: 0.45 },
              { opacity: 0, transform: scale(viewMode) },
            ], { ...motion, fill: "forwards" });
            const enter = incoming?.animate([
              { opacity: 0, transform: scale(nextMode) },
              { opacity: 1, transform: "none" },
            ], motion);
            await Promise.allSettled([exit?.finished, enter?.finished]);
            if (generation !== stackGeneration.current) return;
            flushSync(() => setLeavingMode(null));
            exit?.cancel();
          }
        }

        if (reserve.width !== to.width || reserve.height !== to.height) {
          // Paint the new layout before narrowing the visible and clickable region.
          await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
          if (generation === stackGeneration.current) await invoke("resize_stack_window", to);
        }
      } catch (err) {
        showNotice(String(err));
      } finally {
        switchingView.current = false;
      }
    },
    [engine, hasUnsavedEdits, removing, resetPreviewEditorState, saveCurrentEdits, showNotice, state, targetLang, viewMode]
  );

  switchViewModeRef.current = switchViewMode;

  // Clicking anywhere outside the floating preview window (beyond the 820x560 transparent surface)
  // shifts focus away from the stack window; collapse smoothly back to the compact corner deck.
  useEffect(() => {
    if (viewMode !== "preview" || leavingMode !== null) return;
    const win = getCurrentWebviewWindow();
    let blurTimer: number | null = null;
    let cancelled = false;
    let unlistenFocus: (() => void) | undefined;

    const scheduleBlurCollapse = () => {
      if (blurTimer !== null) window.clearTimeout(blurTimer);
      blurTimer = window.setTimeout(() => {
        blurTimer = null;
        if (
          cancelled ||
          suppressBlurCollapseRef.current ||
          switchingView.current ||
          saveInFlight.current
        ) {
          return;
        }
        void switchViewModeRef.current("collapsed");
      }, 60);
    };

    const cancelBlurCollapse = () => {
      if (blurTimer !== null) {
        window.clearTimeout(blurTimer);
        blurTimer = null;
      }
    };

    const onWindowBlur = () => scheduleBlurCollapse();
    const onWindowFocus = () => cancelBlurCollapse();
    window.addEventListener("blur", onWindowBlur);
    window.addEventListener("focus", onWindowFocus);

    void win
      .onFocusChanged(({ payload: focused }) => {
        if (cancelled) return;
        if (focused) {
          cancelBlurCollapse();
        } else {
          scheduleBlurCollapse();
        }
      })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlistenFocus = fn;
        }
      })
      .catch(console.error);

    return () => {
      cancelled = true;
      cancelBlurCollapse();
      window.removeEventListener("blur", onWindowBlur);
      window.removeEventListener("focus", onWindowFocus);
      if (unlistenFocus) unlistenFocus();
    };
  }, [viewMode, leavingMode]);

  const switchPreviewCard = useCallback(
    async (targetIndex: number, dirOverride?: 1 | -1) => {
      if (!state || state.items.length <= 1) return;
      const len = state.items.length;
      const normalized = ((targetIndex % len) + len) % len;
      if (normalized === activeIndex) return;

      if (saveInFlight.current || removing) return;
      if (hasUnsavedEdits && !(await saveCurrentEdits())) return;

      setSlide(dirOverride ?? (normalized > activeIndex ? 1 : -1));
      resetPreviewEditorState();
      setWorkingImage(null);
      setActiveIndex(normalized);
    },
    [
      activeIndex,
      hasUnsavedEdits,
      removing,
      resetPreviewEditorState,
      saveCurrentEdits,
      state,
    ]
  );

  const commitTextDraft = useCallback(() => {
    setTextDraft((cur) => {
      if (cur && cur.value.trim() && workingImage) {
        setUndoStack((u) => [
          ...u,
          {
            dataUrl: workingImage.dataUrl,
            width: workingImage.width,
            height: workingImage.height,
            annotations,
          },
        ]);
        const fontSize = Math.max(14, Math.round((strokeWidth * 4 + 10) / Math.max(pxScale, 0.25)));
        setAnnotations((prev) => [
          ...prev,
          { kind: "text", color, fontSize, x: cur.x, y: cur.y, text: cur.value.trim() },
        ]);
        setHasUnsavedEdits(true);
      }
      return null;
    });
  }, [annotations, color, pxScale, strokeWidth, workingImage]);

  const applyCrop = useCallback(async () => {
    if (!cropRect || !workingImage || cropRect.w < 12 || cropRect.h < 12) return;
    const flattenedUrl = await flattenWorkingToDataUrl();
    if (!flattenedUrl) return;
    const imgEl = await new Promise<HTMLImageElement>((resolve, reject) => {
      const im = new Image();
      im.onload = () => resolve(im);
      im.onerror = reject;
      im.src = flattenedUrl;
    });
    const sx = Math.max(0, Math.round(cropRect.x));
    const sy = Math.max(0, Math.round(cropRect.y));
    const sw = Math.max(8, Math.min(imgEl.naturalWidth - sx, Math.round(cropRect.w)));
    const sh = Math.max(8, Math.min(imgEl.naturalHeight - sy, Math.round(cropRect.h)));
    const out = document.createElement("canvas");
    out.width = sw;
    out.height = sh;
    const ctx = out.getContext("2d");
    if (!ctx) return;
    ctx.drawImage(imgEl, sx, sy, sw, sh, 0, 0, sw, sh);
    const croppedUrl = out.toDataURL("image/png");

    setUndoStack((u) => [
      ...u,
      {
        dataUrl: workingImage.dataUrl,
        width: workingImage.width,
        height: workingImage.height,
        annotations,
      },
    ]);
    setWorkingImage({ id: workingImage.id, dataUrl: croppedUrl, width: sw, height: sh });
    setAnnotations([]);
    setCropRect(null);
    setTool("pan");
    setZoom(1);
    setPan({ x: 0, y: 0 });
    setHasUnsavedEdits(true);
    showNotice(`已裁剪为 ${sw} × ${sh}，点击保存或切换卡片自动生效`);
  }, [annotations, cropRect, flattenWorkingToDataUrl, showNotice, workingImage]);

  const handleUndo = useCallback(() => {
    if (!activeItem || undoStack.length === 0) return;
    const last = undoStack[undoStack.length - 1];
    setUndoStack((u) => u.slice(0, -1));
    setWorkingImage({ id: activeItem.id, dataUrl: last.dataUrl, width: last.width, height: last.height });
    setAnnotations(last.annotations);
    setCropRect(null);
    setHasUnsavedEdits(true);
  }, [activeItem, undoStack]);

  const handleResetEdits = useCallback(() => {
    if (!activeItem) return;
    const dto = fullImages[activeItem.id];
    const origUrl = dto?.imageDataUrl || activeItem.thumbDataUrl;
    const origW = dto?.pixelWidth || activeItem.pixelWidth;
    const origH = dto?.pixelHeight || activeItem.pixelHeight;
    setWorkingImage({ id: activeItem.id, dataUrl: origUrl, width: origW, height: origH });
    setAnnotations([]);
    setCropRect(null);
    setUndoStack([]);
    setHasUnsavedEdits(false);
    setZoom(1);
    setPan({ x: 0, y: 0 });
  }, [activeItem, fullImages]);

  useEffect(() => {
    if (viewMode === "collapsed") return;
    const onKeyDown = (e: KeyboardEvent) => {
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
      if (activeEl && (activeEl.tagName === "TEXTAREA" || activeEl.tagName === "INPUT" || activeEl.tagName === "SELECT")) {
        if (e.key === "Escape") {
          e.preventDefault();
          switchViewMode("collapsed");
        }
        return;
      }

      if (e.key === "Escape") {
        e.preventDefault();
        if (switchingView.current) return;
        if (viewMode === "preview" && tool === "crop") {
          setCropRect(null);
          setTool("pan");
          return;
        }
        switchViewMode("collapsed");
        return;
      }

      if (viewMode === "preview" && !switchingView.current) {
        if (e.key === "ArrowLeft") {
          e.preventDefault();
          switchPreviewCard(activeIndex - 1, -1);
        } else if (e.key === "ArrowRight") {
          e.preventDefault();
          switchPreviewCard(activeIndex + 1, 1);
        } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
          e.preventDefault();
          handleUndo();
        } else if (e.key === "Enter" && tool === "crop" && cropRect) {
          e.preventDefault();
          applyCrop();
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    activeIndex,
    applyCrop,
    commitTextDraft,
    cropRect,
    handleUndo,
    switchPreviewCard,
    switchViewMode,
    textDraft,
    tool,
    viewMode,
  ]);

  const toImagePoint = (clientX: number, clientY: number): Point | null => {
    const wrap = canvasWrapRef.current;
    if (!wrap) return null;
    const r = wrap.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return null;
    const x = Math.max(0, Math.min(imgW, ((clientX - r.left) / r.width) * imgW));
    const y = Math.max(0, Math.min(imgH, ((clientY - r.top) / r.height) * imgH));
    return { x, y };
  };

  const startPreviewWindowDrag = useCallback(() => {
    suppressBlurCollapseRef.current = true;
    void getCurrentWebviewWindow()
      .startDragging()
      .catch(console.error)
      .finally(() => {
        window.setTimeout(() => {
          suppressBlurCollapseRef.current = false;
        }, 160);
      });
  }, []);

  const onPreviewEdgePointerDown = (e: React.PointerEvent<HTMLButtonElement>) => {
    if (e.button !== 0) return;
    suppressEdgeClickRef.current = false;
    const startX = e.clientX;
    const startY = e.clientY;
    const cleanup = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", cleanup);
      window.removeEventListener("pointercancel", cleanup);
    };
    const onMove = (ev: PointerEvent) => {
      if (Math.hypot(ev.clientX - startX, ev.clientY - startY) > 5) {
        suppressEdgeClickRef.current = true;
        cleanup();
        startPreviewWindowDrag();
      }
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", cleanup);
    window.addEventListener("pointercancel", cleanup);
  };

  const onStageMouseDown = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.button !== 0 || viewMode !== "preview" || saveInFlight.current) return;
    if ((e.target as HTMLElement).closest("button, textarea, input")) return;
    if (tool !== "pan" && !workingImage) {
      showNotice("原图尚未加载，暂时无法编辑");
      return;
    }
    if (textDraft) {
      commitTextDraft();
      return;
    }

    if (tool === "pan") {
      if (zoom > 1 || pan.x !== 0 || pan.y !== 0) {
        dragRef.current = { kind: "pan", sx: e.clientX, sy: e.clientY, orig: { ...pan } };
      } else {
        e.preventDefault();
        startPreviewWindowDrag();
      }
      return;
    }

    const pt = toImagePoint(e.clientX, e.clientY);
    if (!pt) return;

    if (tool === "crop") {
      dragRef.current = { kind: "crop", sx: pt.x, sy: pt.y };
      setCropRect({ x: pt.x, y: pt.y, w: 0, h: 0 });
      return;
    }

    if (tool === "text") {
      setTextDraft({ x: pt.x, y: pt.y, value: "" });
      return;
    }

    dragRef.current = { kind: "draw" };
    const scaledWidth = Math.max(1.5, strokeWidth / Math.max(pxScale, 0.25));
    if (tool === "rect" || tool === "ellipse" || tool === "arrow") {
      setDraftAnnot({ kind: tool, color, width: scaledWidth, x1: pt.x, y1: pt.y, x2: pt.x, y2: pt.y });
    } else if (tool === "pen") {
      setDraftAnnot({ kind: "pen", color, width: scaledWidth, points: [pt] });
    } else if (tool === "mosaic") {
      const blockSize = Math.max(6, Math.round((strokeWidth * 3 + 6) / Math.max(pxScale, 0.25)));
      setDraftAnnot({ kind: "mosaic", blockSize, points: [pt] });
    }
  };

  const onStageMouseMove = (e: React.MouseEvent<HTMLDivElement>) => {
    const mode = dragRef.current;
    if (mode.kind === "none") return;

    if (mode.kind === "pan") {
      setPan({ x: mode.orig.x + (e.clientX - mode.sx), y: mode.orig.y + (e.clientY - mode.sy) });
      return;
    }

    const pt = toImagePoint(e.clientX, e.clientY);
    if (!pt) return;

    if (mode.kind === "crop") {
      setCropRect({
        x: Math.min(mode.sx, pt.x),
        y: Math.min(mode.sy, pt.y),
        w: Math.abs(pt.x - mode.sx),
        h: Math.abs(pt.y - mode.sy),
      });
      return;
    }

    if (mode.kind === "draw") {
      setDraftAnnot((prev) => {
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

  const onStageMouseUp = () => {
    const mode = dragRef.current;
    dragRef.current = { kind: "none" };
    if (mode.kind === "draw") {
      setDraftAnnot((cur) => {
        if (cur && workingImage) {
          setUndoStack((u) => [
            ...u,
            {
              dataUrl: workingImage.dataUrl,
              width: workingImage.width,
              height: workingImage.height,
              annotations,
            },
          ]);
          setAnnotations((prev) => [...prev, cur]);
          setHasUnsavedEdits(true);
        }
        return null;
      });
    }
  };

  const onStageWheel = (e: React.WheelEvent<HTMLDivElement>) => {
    if (viewMode !== "preview") return;
    e.preventDefault();
    const factor = e.deltaY < 0 ? 1.12 : 0.89;
    setZoom((z) => {
      const next = Math.max(0.45, Math.min(4.0, Number((z * factor).toFixed(2))));
      if (next <= 1) setPan({ x: 0, y: 0 });
      return next;
    });
  };

  const handlePreviewCopy = async () => {
    if (!activeItem) return;
    if (hasUnsavedEdits) {
      const ok = await saveCurrentEdits();
      if (!ok) return;
    }
    try {
      await invoke("copy_stack_item_image", { id: activeItem.id });
      markCopied("preview-copy");
      showNotice("已复制当前截图到剪贴板");
    } catch (err) {
      showNotice(String(err));
    }
  };

  const handlePreviewSaveFile = async () => {
    if (!activeItem) return;
    if (hasUnsavedEdits) {
      const ok = await saveCurrentEdits();
      if (!ok) return;
    }
    suppressBlurCollapseRef.current = true;
    try {
      const saved = await invoke<boolean>("save_stack_item_image", { id: activeItem.id });
      if (saved) showNotice("图片已保存");
    } catch (err) {
      showNotice(String(err));
    } finally {
      void getCurrentWebviewWindow().setFocus().catch(() => {});
      window.setTimeout(() => {
        suppressBlurCollapseRef.current = false;
      }, 220);
    }
  };

  const handlePreviewScanQr = async () => {
    if (!activeItem) return;
    const dto = fullImages[activeItem.id];
    const dataUrl = dto?.imageDataUrl ?? activeItem.thumbDataUrl;
    if (!dataUrl) return;
    try {
      const res = await invoke<string[]>("scan_qr_base64", { imageBase64: dataUrl });
      if (res.length === 0) {
        setQrResults(null);
        showNotice("当前图片未检测到二维码");
      } else {
        setQrResults(res);
      }
    } catch (err) {
      showNotice(String(err));
    }
  };

  const handleExportBatchMarkdown = async () => {
    if (!state || state.batchItems.length === 0) return;
    const isOcr = activeTab === "ocr";
    const lines: string[] = [
      `# ${isOcr ? "Luma 批量识字笔记" : "Luma 批量翻译笔记"}`,
      "",
      `> 导出时间：${new Date().toLocaleString()} · 共 ${state.batchItems.length} 张截图`,
      "",
    ];
    for (const it of state.batchItems) {
      lines.push(`## ${it.title}（${it.sourceLang}）`);
      lines.push("");
      if (isOcr) {
        lines.push(it.originalText || it.translatedText || "（未检测到文字）");
      } else {
        if (it.originalText) {
          lines.push("**原文：**");
          lines.push("");
          lines.push(it.originalText);
          lines.push("");
        }
        lines.push("**译文：**");
        lines.push("");
        lines.push(it.translatedText || "（无译文）");
      }
      lines.push("");
    }
    try {
      const saved = await invoke<boolean>("save_markdown_note", {
        content: lines.join("\n"),
        defaultName: isOcr ? "Luma_OCR_Note.md" : "Luma_Translate_Note.md",
      });
      if (saved) showNotice("Markdown 笔记已导出");
    } catch (err) {
      showNotice(String(err));
    }
  };

  const handleExportAskMarkdown = async () => {
    if (!state || state.askMessages.length === 0) return;
    const lines: string[] = [
      "# Luma 多图 AI 分析笔记",
      "",
      `> 导出时间：${new Date().toLocaleString()} · 关联截图 ${state.items.length} 张`,
      "",
    ];
    for (const msg of state.askMessages) {
      lines.push(msg.role === "user" ? "### 提问" : "### AI 回答");
      lines.push("");
      lines.push(msg.content);
      lines.push("");
    }
    try {
      const saved = await invoke<boolean>("save_markdown_note", {
        content: lines.join("\n"),
        defaultName: "Luma_AI_Note.md",
      });
      if (saved) showNotice("Markdown 笔记已导出");
    } catch (err) {
      showNotice(String(err));
    }
  };

  const startThumbDrag = (
    e: React.PointerEvent<HTMLDivElement>,
    fromIdx: number
  ) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest(".stack-thumb-btn")) return;
    e.preventDefault();
    const stripEl = e.currentTarget.parentElement;
    const startX = e.clientX;
    const startY = e.clientY;
    const itemCount = state?.items.length ?? 0;
    // Snapshot resting item midpoints once before ripple transforms shift layout rects.
    const initialStrip = stripEl ?? thumbStripRef.current;
    const initialScrollLeft = initialStrip?.scrollLeft ?? 0;
    const restingMidpoints = initialStrip
      ? Array.from(initialStrip.querySelectorAll<HTMLElement>(".stack-thumb-item")).map((el) => {
          const r = el.getBoundingClientRect();
          return r.left + r.width / 2;
        })
      : [];
    let moved = false;
    let currentSlot = fromIdx;
    let cancelled = false;

    const calcDropSlot = (clientX: number) => {
      const strip = stripEl ?? thumbStripRef.current;
      const scrollDelta = strip ? strip.scrollLeft - initialScrollLeft : 0;
      const adjustedX = clientX + scrollDelta;
      if (restingMidpoints.length > 0) {
        for (let i = 0; i < restingMidpoints.length; i++) {
          if (adjustedX < restingMidpoints[i]) {
            return i;
          }
        }
        return restingMidpoints.length;
      }
      if (!strip) return fromIdx;
      const children = Array.from(strip.querySelectorAll<HTMLElement>(".stack-thumb-item"));
      for (let i = 0; i < children.length; i++) {
        const rect = children[i].getBoundingClientRect();
        if (clientX < rect.left + rect.width / 2) {
          return i;
        }
      }
      return children.length;
    };

    const isOutsideStrip = (clientX: number, clientY: number) => {
      const strip = stripEl ?? thumbStripRef.current;
      if (!strip) return false;
      const r = strip.getBoundingClientRect();
      const pad = 42;
      return (
        clientX < r.left - pad ||
        clientX > r.right + pad ||
        clientY < r.top - pad ||
        clientY > r.bottom + pad
      );
    };

    const cleanup = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      window.removeEventListener("keydown", onKeyDown, true);
    };

    const onMove = (ev: PointerEvent) => {
      if (cancelled) return;
      const dx = ev.clientX - startX;
      const dy = ev.clientY - startY;
      if (!moved && Math.hypot(dx, dy) > 6) {
        moved = true;
      }
      if (!moved) return;
      currentSlot = isOutsideStrip(ev.clientX, ev.clientY)
        ? fromIdx
        : calcDropSlot(ev.clientX);
      setDragReorder({
        fromIdx,
        dropSlot: currentSlot,
        dx,
        dy,
        active: true,
      });
    };

    const onCancel = () => {
      cancelled = true;
      cleanup();
      setDragReorder(null);
    };

    const onKeyDown = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") {
        ev.preventDefault();
        ev.stopPropagation();
        onCancel();
      }
    };

    const onUp = (ev: PointerEvent) => {
      cleanup();
      setDragReorder(null);
      if (cancelled) return;
      if (!moved) {
        switchViewMode("preview", fromIdx);
        return;
      }
      if (isOutsideStrip(ev.clientX, ev.clientY)) return;
      const toIndex = currentSlot > fromIdx ? currentSlot - 1 : currentSlot;
      if (toIndex !== fromIdx && toIndex >= 0 && toIndex < itemCount) {
        const movedId = state?.items[fromIdx]?.id ?? null;
        if (movedId !== null) {
          setJustDroppedId(movedId);
          if (dropSettleTimerRef.current !== null) {
            window.clearTimeout(dropSettleTimerRef.current);
          }
          dropSettleTimerRef.current = window.setTimeout(() => {
            dropSettleTimerRef.current = null;
            setJustDroppedId(null);
          }, 360);
        }
        invoke("reorder_stack_items", { fromIndex: fromIdx, toIndex }).catch((err) =>
          showNotice(String(err))
        );
      }
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    window.addEventListener("keydown", onKeyDown, true);
  };

  const renderThumbItems = () => {
    const items = state?.items ?? [];
    const activeDrag = dragReorder?.active ? dragReorder : null;
    const showHint =
      activeDrag !== null &&
      activeDrag.dropSlot !== activeDrag.fromIdx &&
      activeDrag.dropSlot !== activeDrag.fromIdx + 1;
    const dodgeMag = [10, 7, 5, 3];

    return items.map((item, idx) => {
      const isDragging = activeDrag !== null && activeDrag.fromIdx === idx;
      const isJustDropped = !activeDrag && justDroppedId === item.id;
      const isNeighborNudge = !activeDrag && Boolean(nudgeThumbIds[item.id]);

      let rippleStyle: React.CSSProperties | undefined;
      if (activeDrag) {
        if (isDragging) {
          // Lift and slightly scale down so the thin drop indicator line remains unobstructed (Vibe 005).
          const liftY = Math.max(-10, Math.min(4, activeDrag.dy - 6));
          rippleStyle = {
            cursor: "grabbing",
            transform: `translate3d(${activeDrag.dx}px, ${liftY}px, 0) scale(0.95)`,
            zIndex: 20,
            pointerEvents: "none",
            transition: "box-shadow 140ms ease, opacity 140ms ease",
          };
        } else if (showHint) {
          const slot = activeDrag.dropSlot;
          const isLeft = idx < slot;
          const n = isLeft ? slot - 1 - idx : idx - slot;
          const shift = dodgeMag[Math.min(n, dodgeMag.length - 1)];
          rippleStyle = {
            cursor: "grabbing",
            transform: `translate3d(${isLeft ? -shift : shift}px, 0, 0)`,
            transitionDelay: `${n * 24}ms`,
          };
        }
      } else {
        rippleStyle = { cursor: "grab" };
      }

      const hintBefore = showHint && activeDrag?.dropSlot === idx;
      const hintAfter = showHint && idx === items.length - 1 && activeDrag?.dropSlot === items.length;
      const removingStyle = removingThumbIds[item.id];
      const isJustRestored = !activeDrag && justRestoredId === item.id;

      return (
        <React.Fragment key={item.id}>
          {hintBefore && <span className="stack-drop-hint" aria-hidden="true" />}
          <div
            className={`stack-thumb-item${isDragging ? " dragging" : ""}${removingStyle === "peel" ? " removing peel-out" : removingStyle === "blocks" ? " removing blocks-out" : ""}${isJustDropped ? " just-dropped" : ""}${isJustRestored ? " just-restored" : ""}${isNeighborNudge ? " neighbor-nudge" : ""}`}
            tabIndex={0}
            aria-label={`截图 #${idx + 1}，拖拽或 ${ALT_LABEL}+方向键排序`}
            onPointerDown={(e) => startThumbDrag(e, idx)}
            onKeyDown={(e) => {
              if (e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
                const toIndex = e.key === "ArrowLeft" ? idx - 1 : idx + 1;
                if (toIndex >= 0 && toIndex < items.length) {
                  e.preventDefault();
                  e.stopPropagation();
                  invoke("reorder_stack_items", { fromIndex: idx, toIndex }).catch((err) =>
                    showNotice(String(err))
                  );
                }
              } else if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                switchViewMode("preview", idx);
              }
            }}
            style={rippleStyle}
            title={`点击预览 · 拖拽或 ${ALT_LABEL}+方向键排序`}
          >
            <img src={item.thumbDataUrl} alt={item.title} draggable={false} />
            <span className="stack-thumb-num">#{idx + 1}</span>
            <div className="stack-thumb-overlay">
                <button
                  type="button"
                  className="stack-thumb-btn"
                  onClick={(e) => {
                    e.stopPropagation();
                    invoke("pin_stack_item", { id: item.id }).catch((err) => showNotice(String(err)));
                  }}
                  title="贴图"
                >
                  <Pin size={11} />
                </button>
                <button
                  type="button"
                  className="stack-thumb-btn danger"
                  disabled={Boolean(removingStyle)}
                  onClick={(e) => {
                    e.stopPropagation();
                    void handleThumbRemove(item.id);
                  }}
                  title="移出（可撤销）"
                >
                  <X size={11} />
                </button>
              </div>
          </div>
          {hintAfter && <span className="stack-drop-hint" aria-hidden="true" />}
        </React.Fragment>
      );
    });
  };

  // Vibe 005: Vertical Drag-Ripple Reorder (lifted card shifts right 10px & scales to 94%, neighbors dodge 10->7->5->3px with 24ms ripple)
  const startVerticalCardDrag = (
    e: React.PointerEvent<HTMLDivElement>,
    fromIdx: number
  ) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button")) return;
    e.preventDefault();
    const listEl = dragListRef.current;
    const itemCount = state?.items.length ?? 0;
    const startX = e.clientX;
    const startY = e.clientY;
    const initialScrollTop = listEl?.scrollTop ?? 0;
    const listRect = listEl?.getBoundingClientRect();
    const restingCards = listEl && listRect
      ? Array.from(listEl.querySelectorAll<HTMLElement>(".stack-drag-card")).map((el) => {
          const r = el.getBoundingClientRect();
          return {
            relTop: r.top - listRect.top + initialScrollTop,
            height: r.height,
            midClientY: r.top + r.height / 2,
          };
        })
      : [];

    let moved = false;
    let currentSlot = fromIdx;
    let cancelled = false;

    const calcSlotAndLine = (clientY: number) => {
      const scrollDelta = listEl ? listEl.scrollTop - initialScrollTop : 0;
      const adjustedY = clientY + scrollDelta;
      let slot = restingCards.length;
      for (let i = 0; i < restingCards.length; i++) {
        if (adjustedY < restingCards[i].midClientY) {
          slot = i;
          break;
        }
      }
      let lineTop = 6;
      if (restingCards.length > 0) {
        if (slot <= 0) {
          lineTop = Math.max(4, restingCards[0].relTop - 5);
        } else if (slot >= restingCards.length) {
          const last = restingCards[restingCards.length - 1];
          lineTop = last.relTop + last.height + 4;
        } else {
          lineTop = restingCards[slot].relTop - 5;
        }
      }
      return { slot, lineTop };
    };

    const cleanup = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      window.removeEventListener("keydown", onKeyDown, true);
    };

    const onMove = (ev: PointerEvent) => {
      if (cancelled) return;
      const dx = ev.clientX - startX;
      const dy = ev.clientY - startY;
      if (!moved && Math.hypot(dx, dy) > 4) {
        moved = true;
      }
      if (!moved) return;
      const { slot, lineTop } = calcSlotAndLine(ev.clientY);
      currentSlot = slot;
      setVerticalDrag({
        fromIdx,
        dropSlot: slot,
        dy,
        lineTop,
        active: true,
      });
    };

    const onCancel = () => {
      cancelled = true;
      cleanup();
      setVerticalDrag(null);
    };

    const onKeyDown = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") {
        ev.preventDefault();
        ev.stopPropagation();
        onCancel();
      }
    };

    const onUp = () => {
      cleanup();
      setVerticalDrag(null);
      if (cancelled) return;
      if (!moved) {
        void switchViewMode("preview", fromIdx);
        return;
      }
      const toIndex = currentSlot > fromIdx ? currentSlot - 1 : currentSlot;
      if (toIndex !== fromIdx && toIndex >= 0 && toIndex < itemCount) {
        const movedId = state?.items[fromIdx]?.id ?? null;
        if (movedId !== null) {
          setJustDroppedId(movedId);
          if (dropSettleTimerRef.current !== null) {
            window.clearTimeout(dropSettleTimerRef.current);
          }
          dropSettleTimerRef.current = window.setTimeout(() => {
            dropSettleTimerRef.current = null;
            setJustDroppedId(null);
          }, 380);
        }
        invoke("reorder_stack_items", { fromIndex: fromIdx, toIndex })
          .then(() => showNotice("已调整叠堆顺序"))
          .catch((err) => showNotice(String(err)));
      }
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    window.addEventListener("keydown", onKeyDown, true);
  };

  const renderVerticalDragList = () => {
    const items = state?.items ?? [];
    const activeDrag = verticalDrag?.active ? verticalDrag : null;
    const showLine =
      activeDrag !== null &&
      activeDrag.dropSlot !== activeDrag.fromIdx &&
      activeDrag.dropSlot !== activeDrag.fromIdx + 1;
    const dodgeMag = [10, 7, 5, 3];

    return (
      <div className="stack-drag-list" ref={dragListRef}>
        <div
          className={`stack-drop-line${showLine ? " show" : ""}`}
          style={{ top: `${activeDrag?.lineTop ?? 0}px` }}
          aria-hidden="true"
        />
        {items.map((item, idx) => {
          const isDragging = activeDrag !== null && activeDrag.fromIdx === idx;
          const isJustDropped = !activeDrag && justDroppedId === item.id;
          const isJustRestored = !activeDrag && justRestoredId === item.id;
          const isNeighborNudge = !activeDrag && Boolean(nudgeThumbIds[item.id]);
          const removingStyle = removingThumbIds[item.id];

          let cardStyle: React.CSSProperties | undefined;
          if (activeDrag) {
            if (isDragging) {
              // Pull aside 10px & shrink to 94% per Vibe 005 so drop target line stays clearly visible
              cardStyle = {
                cursor: "grabbing",
                transform: `translate3d(10px, ${activeDrag.dy}px, 0) scale(0.94)`,
                zIndex: 30,
                pointerEvents: "none",
                transition: "box-shadow 140ms ease, border-color 140ms ease",
              };
            } else if (showLine) {
              const slot = activeDrag.dropSlot;
              const isAbove = idx < slot;
              const dist = Math.max(0, isAbove ? slot - 1 - idx : idx - slot);
              const mag = dodgeMag[Math.min(dist, dodgeMag.length - 1)];
              cardStyle = {
                transform: `translate3d(0, ${isAbove ? -mag : mag}px, 0)`,
                transitionDelay: `${dist * 24}ms`,
              };
            }
          }

          return (
            <div
              key={item.id}
              className={`stack-drag-card${isDragging ? " dragging" : ""}${removingStyle === "peel" ? " peel-out" : removingStyle === "blocks" ? " blocks-out" : ""}${isNeighborNudge ? " neighbor-nudge" : ""}${isJustDropped ? " just-dropped" : ""}${isJustRestored ? " just-restored" : ""}`}
              style={cardStyle}
              tabIndex={0}
              aria-label={`#${idx + 1} ${item.title}，拖拽或 ${ALT_LABEL}+上下方向键排序`}
              onPointerDown={(e) => startVerticalCardDrag(e, idx)}
              onKeyDown={(e) => {
                if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
                  const toIndex = e.key === "ArrowUp" ? idx - 1 : idx + 1;
                  if (toIndex >= 0 && toIndex < items.length) {
                    e.preventDefault();
                    e.stopPropagation();
                    invoke("reorder_stack_items", { fromIndex: idx, toIndex }).catch((err) =>
                      showNotice(String(err))
                    );
                  }
                } else if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  void switchViewMode("preview", idx);
                } else if (e.key === "Delete" || e.key === "Backspace") {
                  e.preventDefault();
                  void handleThumbRemove(item.id);
                }
              }}
              title={`拖起排序 · 点击预览编辑 · ${ALT_LABEL}+↑/↓ 排序`}
            >
              <div className="stack-drag-grip" aria-hidden="true">
                <GripVertical size={14} />
              </div>
              <div className="stack-drag-thumb">
                <img src={item.thumbDataUrl} alt={item.title} draggable={false} />
              </div>
              <div className="stack-drag-main">
                <strong>
                  #{idx + 1} · {item.title}
                </strong>
                <span>
                  {item.pixelWidth}×{item.pixelHeight} px · 点击预览与标注
                </span>
              </div>
              <div className="stack-drag-actions">
                <button
                  type="button"
                  className="stack-card-icon-btn subtle"
                  onClick={(e) => {
                    e.stopPropagation();
                    void switchViewMode("preview", idx);
                  }}
                  title="预览与标注"
                  aria-label="预览与标注"
                >
                  <Eye size={13} />
                </button>
                <button
                  type="button"
                  className="stack-card-icon-btn subtle"
                  onClick={(e) => {
                    e.stopPropagation();
                    invoke("pin_stack_item", { id: item.id }).catch((err) => showNotice(String(err)));
                  }}
                  title="贴图到屏幕"
                  aria-label="贴图到屏幕"
                >
                  <Pin size={13} />
                </button>
                <button
                  type="button"
                  className="stack-card-icon-btn remove"
                  disabled={Boolean(removingStyle)}
                  onClick={(e) => {
                    e.stopPropagation();
                    void handleThumbRemove(item.id);
                  }}
                  title={deleteStyle === "peel" ? "撕贴纸移出（可撤销）" : "Blocks 消散移出（可撤销）"}
                  aria-label={`移出 #${idx + 1} ${item.title}`}
                >
                  <X size={13} />
                </button>
              </div>
            </div>
          );
        })}
      </div>
    );
  };

  const handleThumbRemove = async (id: number) => {
    if (removingThumbIds[id]) return;
    const reduced =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    const items = state?.items ?? [];
    const idx = items.findIndex((it) => it.id === id);
    const targetItem = idx >= 0 ? items[idx] : null;
    const styleUsed = deleteStyle;

    // React immediate neighbors BEFORE card finishes leaving (Vibe 014)
    if (!reduced && idx >= 0) {
      const neighbors: Record<number, true> = {};
      if (items[idx - 1]) neighbors[items[idx - 1].id] = true;
      if (items[idx + 1]) neighbors[items[idx + 1].id] = true;
      if (Object.keys(neighbors).length > 0) {
        setNudgeThumbIds(neighbors);
        if (nudgeTimerRef.current !== null) {
          window.clearTimeout(nudgeTimerRef.current);
        }
        nudgeTimerRef.current = window.setTimeout(() => {
          nudgeTimerRef.current = null;
          setNudgeThumbIds({});
        }, 360);
      }
    }
    setRemovingThumbIds((prev) => ({ ...prev, [id]: styleUsed }));
    if (!reduced) {
      await new Promise((resolve) =>
        window.setTimeout(resolve, styleUsed === "peel" ? 280 : 220)
      );
    }
    try {
      await invoke("remove_stack_item", { id });
      setFullImages((images) => {
        if (!(id in images)) return images;
        const next = { ...images };
        delete next[id];
        return next;
      });
      if (targetItem) {
        showUndoToast(targetItem.title, `#${idx + 1}`);
      }
    } catch (err) {
      showNotice(String(err));
    } finally {
      setRemovingThumbIds((prev) => {
        if (!(id in prev)) return prev;
        const next = { ...prev };
        delete next[id];
        return next;
      });
    }
  };

  const handlePreviewPin = async () => {
    if (!activeItem) return;
    if (hasUnsavedEdits) {
      const ok = await saveCurrentEdits();
      if (!ok) return;
    }
    invoke("pin_stack_item", { id: activeItem.id }).catch((err) => showNotice(String(err)));
  };

  const handlePreviewRemove = async () => {
    if (!activeItem || removing || saveInFlight.current) return;
    const id = activeItem.id;
    const title = activeItem.title;
    const idxLabel = `#${activeIndex + 1}`;
    setRemoving(true);
    try {
      await invoke("remove_stack_item", { id });
      setSlide(null);
      resetPreviewEditorState();
      setFullImages((images) => {
        const next = { ...images };
        delete next[id];
        return next;
      });
      showUndoToast(title, idxLabel);
    } catch (err) {
      showNotice(String(err));
    } finally {
      setRemoving(false);
    }
  };

  const handleCopyStitched = async () => {
    try {
      const count = await invoke<number>("copy_stitched_stack", { direction });
      showNotice(`已拼接 ${count} 张并复制到剪贴板`);
      markCopied("stitch-copy");
    } catch (err) {
      showNotice(String(err));
    }
  };

  const handleSaveStitched = async () => {
    try {
      const saved = await invoke<boolean>("save_stitched_stack", { direction });
      if (saved) showNotice("拼图已保存");
    } catch (err) {
      showNotice(String(err));
    }
  };

  const handleCopyAsFiles = async () => {
    try {
      const count = await invoke<number>("copy_stack_as_files");
      showNotice(`已复制 ${count} 张图片文件，可直接 ${MOD_LABEL}+V 粘贴`);
      markCopied("files");
    } catch (err) {
      showNotice(String(err));
    }
  };

  const handleCollapsedCopyAll = async () => {
    if (!state || state.items.length === 0) return;
    try {
      if (state.items.length === 1) {
        await invoke("copy_stack_item_image", { id: state.items[0].id });
        markCopied("collapsed-copy");
        showNotice("已复制截图");
      } else {
        const count = await invoke<number>("copy_stack_as_files");
        markCopied("collapsed-copy");
        showNotice(`已复制 ${count} 张截图`);
      }
    } catch (err) {
      showNotice(String(err));
    }
  };

  const switchBatchTab = (mode: "translate" | "ocr") => {
    setActiveTab(mode);
    const hasCurrentBatch = state?.batchMode === mode &&
      (state.batchLoading || state.batchItems.length > 0);
    if (!hasCurrentBatch) {
      runBatch(mode);
    }
  };

  const runBatch = async (mode: "translate" | "ocr", engOverride?: TranslateEngine, langOverride?: string) => {
    setActiveTab(mode);
    try {
      await invoke("stack_run_batch", {
        mode,
        engineOverride: engOverride ?? engine,
        targetLangOverride: langOverride ?? targetLang,
      });
    } catch (err) {
      showNotice(String(err));
    }
  };

  const sendAsk = async (prompt?: string) => {
    const q = (prompt ?? askInput).trim();
    if (!q || state?.askLoading || askSubmitting.current) return;
    askSubmitting.current = true;
    try {
      await invoke("stack_ask_ai", { question: q });
      if (!prompt) setAskInput((draft) => draft.trim() === q ? "" : draft);
    } catch (err) {
      showNotice(String(err));
    } finally {
      askSubmitting.current = false;
    }
  };

  const copyText = async (text: string, key: string) => {
    await navigator.clipboard.writeText(text);
    markCopied(key);
  };

  const renderUndoToast = (variant: "workspace" | "floating" = "workspace") => {
    if (!undoToast) return null;
    return (
      <div
        className={`stack-undo-toast${variant === "floating" ? " is-floating" : ""}`}
        role="status"
        aria-live="polite"
        onMouseEnter={() => {
          undoToastHoveredRef.current = true;
          clearUndoTimer();
        }}
        onMouseLeave={() => {
          undoToastHoveredRef.current = false;
          scheduleUndoDismiss();
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <span className="stack-undo-dot" aria-hidden="true" />
        <span className="stack-undo-msg" title={`已移出「${undoToast.indexLabel} ${undoToast.title}」`}>
          已移出「{undoToast.indexLabel} {undoToast.title}」
        </span>
        <button
          type="button"
          className="stack-undo-btn"
          disabled={undoRestoring}
          onClick={() => void handleUndoRemove()}
        >
          <Undo2 size={11} /> 撤销
        </button>
        <button
          type="button"
          className="stack-undo-close"
          onClick={handleDismissUndoToast}
          title="关闭提示"
          aria-label="关闭提示"
        >
          <X size={11} />
        </button>
      </div>
    );
  };

  if (!state || state.items.length === 0) {
    if (undoToast) {
      return (
        <div className="stack-view-surface">
          <div className="stack-mode-layer" style={COLLAPSED_SIZE}>
            <div className="stack-empty-undo-shell">{renderUndoToast("floating")}</div>
          </div>
        </div>
      );
    }
    return null;
  }

  // ==================== 1. Corner Physical Card Fan (Shared by Collapsed & Expanded Drawer) ====================
  const renderCornerDeck = (isExpanded: boolean) => {
    const visible = state.items.slice(-4);
    const total = state.items.length;
    const baseIndex = total - visible.length;

    return (
      <div className="stack-corner-anchor">
        <div
          className={`corner-stack${catchBounce ? " catch-bounce" : ""}${isExpanded ? " is-expanded" : ""}`}
          role="button"
          tabIndex={0}
          aria-expanded={isExpanded}
          aria-label={
            isExpanded
              ? `收起叠堆浮层，当前 ${total}/${state.maxStackSize} 张`
              : `展开叠堆浮层，当前 ${total}/${state.maxStackSize} 张`
          }
          onClick={() =>
            void switchViewMode(
              isExpanded ? "collapsed" : "workspace",
              undefined,
              isExpanded ? undefined : "cards"
            )
          }
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              void switchViewMode(
                isExpanded ? "collapsed" : "workspace",
                undefined,
                isExpanded ? undefined : "cards"
              );
            }
          }}
          title={isExpanded ? "点击收起叠堆浮层" : "点击展开叠堆浮层"}
        >
          {visible.map((item, idx) => {
            const offset = visible.length - 1 - idx;
            const itemNum = baseIndex + idx + 1;
            const layerClass =
              offset === 0 ? "l1" : offset === 1 ? "l2" : offset === 2 ? "l3" : "l4";

            if (offset > 0) {
              return (
                <div
                  key={item.id}
                  className={`corner-stack-card back ${layerClass}`}
                  aria-hidden="true"
                >
                  <div className="corner-card-media">
                    <img src={item.thumbDataUrl} alt={item.title} draggable={false} />
                    <span className="corner-back-num">#{itemNum}</span>
                  </div>
                </div>
              );
            }

            return (
              <div key={item.id} className="corner-stack-card front l1">
                <div className="corner-card-media">
                  <img src={item.thumbDataUrl} alt={item.title} draggable={false} />

                  <span
                    className="corner-drag-grip"
                    data-tauri-drag-region
                    onClick={(e) => e.stopPropagation()}
                    title="按住拖动叠堆窗口位置"
                  >
                    <GripVertical size={12} />
                  </span>

                  <span className="stack-slot-badge" aria-label={`${total} / ${state.maxStackSize}`}>
                    <span key={countRollKey} className="slot-digit in">
                      {total}
                    </span>
                    <span className="slot-sep">/{state.maxStackSize}</span>
                    <span className="corner-badge-chevron" aria-hidden="true">
                      {isExpanded ? <ChevronDown size={10} /> : <ChevronUp size={10} />}
                    </span>
                  </span>

                  <div
                    className="corner-card-micro-actions"
                    onClick={(e) => e.stopPropagation()}
                  >
                    <button
                      type="button"
                      className={`corner-micro-btn${copiedKey === "collapsed-copy" ? " copied" : ""}`}
                      onClick={() => void handleCollapsedCopyAll()}
                      title={total > 1 ? `复制全部 ${total} 张` : "复制截图"}
                      aria-label="复制全部截图"
                    >
                      {copiedKey === "collapsed-copy" ? <Check size={12} /> : <Copy size={12} />}
                    </button>
                    <button
                      type="button"
                      className="corner-micro-btn"
                      onClick={() =>
                        invoke("start_capture", { hideMain: false }).catch((err) =>
                          showNotice(String(err))
                        )
                      }
                      title="继续截图"
                      aria-label="继续截图"
                    >
                      <Camera size={12} />
                    </button>
                    <button
                      type="button"
                      className="corner-micro-btn danger"
                      onClick={() => invoke("clear_stack").catch(console.error)}
                      title="清空叠堆"
                      aria-label="清空叠堆"
                    >
                      <X size={12} />
                    </button>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      </div>
    );
  };

  const renderCollapsed = () => (
    <div className="stack-collapsed-shell" style={COLLAPSED_SIZE}>
      {undoToast ? (
        <div className="stack-collapsed-undo-wrap">{renderUndoToast("floating")}</div>
      ) : (
        feedback && <div className="stack-collapsed-toast">{feedback}</div>
      )}
      {renderCornerDeck(false)}
    </div>
  );

  // ==================== 2. 3D Layered Deck Preview & Floating Editor ====================
  const renderPreview = () => {
    if (!activeItem) return null;
    const backLayers = state.items.filter((item) => item.id !== activeItem.id).slice(-4);

    return (
      <div
        className="stack-preview-shell"
        onPointerDownCapture={(event) => {
          const inContent = (event.target as Element).closest("[data-preview-content]");
          backdropPressRef.current = event.button === 0 && !inContent
            ? { x: event.clientX, y: event.clientY }
            : null;
        }}
        onPointerUpCapture={(event) => {
          const hit = document.elementFromPoint(event.clientX, event.clientY);
          if (!hit || hit.closest("[data-preview-content]")) {
            backdropPressRef.current = null;
          }
        }}
        onPointerCancel={() => { backdropPressRef.current = null; }}
        onClick={(event) => {
          const press = backdropPressRef.current;
          backdropPressRef.current = null;
          if (press && Math.hypot(event.clientX - press.x, event.clientY - press.y) <= 6 &&
              !(event.target as Element).closest("[data-preview-content]")) {
            void switchViewMode("collapsed");
          }
        }}
      >
        <div ref={viewportRef} className="stack-preview-spatial-stage">
          {undoToast ? (
            <div className="stack-preview-undo-wrap" data-preview-content>
              {renderUndoToast("floating")}
            </div>
          ) : (
            feedback && <div className="stack-floating-notice">{feedback}</div>
          )}

          <div
            className="stack-preview-stage"
            onWheel={onStageWheel}
          >
            <div
              className="stack-preview-deck-cluster"
              style={{
                width: Math.max(paperBoxW, 200),
                height: Math.max(paperBoxH, 100),
              }}
            >
              {backLayers.map((item, layerIdx) => {
                const depth = backLayers.length - layerIdx;
                const full = fullImages[item.id];
                const itemW = Math.max(full?.pixelWidth || item.pixelWidth || 1, 1);
                const itemH = Math.max(full?.pixelHeight || item.pixelHeight || 1, 1);
                const backFit = Math.min(stageAvailW / itemW, stageAvailH / itemH, 1.35);
                const targetScale = Math.max(0.84, 0.96 - depth * 0.04);
                const backCardW = Math.min(
                  stageAvailW - 12,
                  Math.max(168, Math.round((paperBoxW * 0.72 + itemW * backFit * 0.28) * targetScale))
                );
                const backCardH = Math.min(
                  stageAvailH - 12,
                  Math.max(82, Math.round((paperBoxH * 0.72 + itemH * backFit * 0.28) * targetScale))
                );
                const offsetX = (depth % 2 === 1 ? -1 : 1) * (20 + depth * 12);
                const offsetY = -depth * (paperBoxH < 130 ? 12 : 9);
                const rot = (depth % 2 === 1 ? -1 : 1) * (2.8 + depth * 1.25);
                const backBrightness = Math.max(0.76, 1 - depth * 0.08);
                return (
                  <div
                    key={item.id}
                    className="stack-preview-3d-back-card"
                    aria-hidden="true"
                    style={{
                      zIndex: 1 + layerIdx,
                      width: backCardW,
                      height: backCardH,
                      transform: `translate3d(${offsetX}px, ${offsetY}px, ${-depth * 36}px) rotateZ(${rot}deg) rotateX(2deg)`,
                      filter: `brightness(${backBrightness})`,
                    }}
                  >
                    <img
                      src={full?.imageDataUrl || item.thumbDataUrl}
                      alt={item.title}
                      draggable={false}
                    />
                  </div>
                );
              })}

              <div
                key={activeItem.id}
                className="stack-preview-paper"
                data-preview-content
                data-narrow={paperBoxW < 110 || undefined}
                data-slide={slide === null ? undefined : slide === 1 ? "next" : "previous"}
                onMouseDown={onStageMouseDown}
                onMouseMove={onStageMouseMove}
                onMouseUp={onStageMouseUp}
                onMouseLeave={onStageMouseUp}
                style={{
                  cursor:
                    tool === "pan"
                      ? "grab"
                      : tool === "text"
                        ? "text"
                        : "crosshair",
                }}
              >
                <div
                  ref={canvasWrapRef}
                  className="stack-preview-canvas-wrap"
                  style={{
                    width: dispW,
                    height: dispH,
                    transform: `translate3d(${pan.x}px, ${pan.y}px, 0)`,
                  }}
                >
                  <img
                    src={workingImage?.dataUrl || fullImages[activeItem.id]?.imageDataUrl || activeItem.thumbDataUrl}
                    alt={activeItem.title}
                    className="stack-preview-img"
                    draggable={false}
                  />
                  <canvas ref={annotCanvasRef} className="stack-preview-annot-canvas" />

                  {cropRect && cropRect.w > 2 && cropRect.h > 2 && (
                    <>
                      <svg className="stack-crop-mask" viewBox={`0 0 ${imgW} ${imgH}`} preserveAspectRatio="none">
                        <defs>
                          <mask id="stack-crop-cutout">
                            <rect width={imgW} height={imgH} fill="white" />
                            <rect x={cropRect.x} y={cropRect.y} width={cropRect.w} height={cropRect.h} fill="black" />
                          </mask>
                        </defs>
                        <rect width={imgW} height={imgH} fill="rgba(0,0,0,0.48)" mask="url(#stack-crop-cutout)" />
                      </svg>
                      <div
                        className="stack-crop-box"
                        style={{
                          left: cropRect.x * pxScale,
                          top: cropRect.y * pxScale,
                          width: cropRect.w * pxScale,
                          height: cropRect.h * pxScale,
                        }}
                      >
                        <span className="stack-crop-dim-tag">
                          {Math.round(cropRect.w)} × {Math.round(cropRect.h)}
                        </span>
                      </div>
                    </>
                  )}

                  {textDraft && (
                    <textarea
                      ref={textInputRef}
                      className="overlay-text-input"
                      rows={1}
                      style={{
                        left: textDraft.x * pxScale,
                        top: textDraft.y * pxScale,
                        color,
                        fontSize: Math.max(14, strokeWidth * 4 + 10),
                      }}
                      value={textDraft.value}
                      onChange={(e) =>
                        setTextDraft((prev) => (prev ? { ...prev, value: e.target.value } : null))
                      }
                      onBlur={commitTextDraft}
                      placeholder="输入文字，Enter 确认"
                    />
                  )}
                </div>
                {tool === "pan" && zoom <= 1 && state.items.length > 1 && (
                  <>
                    <button
                      type="button"
                      className="stack-preview-edge previous"
                      aria-label="上一张"
                      title="上一张 (←)"
                      onPointerDown={onPreviewEdgePointerDown}
                      onClick={() => {
                        if (suppressEdgeClickRef.current) {
                          suppressEdgeClickRef.current = false;
                          return;
                        }
                        void switchPreviewCard(activeIndex - 1, -1);
                      }}
                    />
                    <button
                      type="button"
                      className="stack-preview-edge next"
                      aria-label="下一张"
                      title="下一张 (→)"
                      onPointerDown={onPreviewEdgePointerDown}
                      onClick={() => {
                        if (suppressEdgeClickRef.current) {
                          suppressEdgeClickRef.current = false;
                          return;
                        }
                        void switchPreviewCard(activeIndex + 1, 1);
                      }}
                    />
                  </>
                )}
                <span className="stack-preview-index" aria-live="polite">
                  {activeIndex + 1}/{state.items.length}
                </span>
                <div className="stack-preview-corners" onMouseDown={(event) => event.stopPropagation()}>
                  <button type="button" className="hud-icon-btn" onClick={handlePreviewPin} aria-label="置顶贴图" title="置顶贴图">
                    <Pin size={13} />
                  </button>
                  <button type="button" className="hud-icon-btn remove" onClick={handlePreviewRemove} disabled={removing || savingEdits} aria-label="删除这张截图" title="删除这张截图">
                    <X size={14} />
                  </button>
                </div>
              </div>
            </div>

            {tool === "crop" && cropRect && cropRect.w >= 12 && cropRect.h >= 12 && (
              <div className="stack-crop-confirm-bar" data-preview-content>
                <button type="button" className="btn btn-primary btn-sm" onClick={applyCrop}>
                  <Check size={12} /> 确认裁剪
                </button>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => {
                    setCropRect(null);
                    setTool("pan");
                  }}
                >
                  取消
                </button>
              </div>
            )}
          </div>

          {/* Stereoscopically Floating Editor Dock */}
          <div className="stack-floating-dock" data-preview-content>
            {showAnnotSubbar && (
              <div className="stack-preview-subbar">
                {tool !== "mosaic" && (
                  <div className="stack-subbar-group">
                    {PREVIEW_COLORS.map((c) => (
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
                )}
                <div className="stack-subbar-group">
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
              </div>
            )}

            <div className="stack-preview-toolbar">
              <div
                className="stack-preview-drag-grip"
                data-tauri-drag-region
                onMouseDown={(e) => {
                  if (e.button === 0) {
                    e.preventDefault();
                    startPreviewWindowDrag();
                  }
                }}
                title="按住拖动预览窗口位置"
                aria-label="拖动预览窗口位置"
              >
                <GripVertical size={13} />
              </div>
              <div className="stack-tool-group">
                <button
                  type="button"
                  className={`hud-icon-btn ${tool === "pan" ? "active" : ""}`}
                  onClick={() => {
                    setCropRect(null);
                    setTool("pan");
                  }}
                  title="浏览 / 拖拽平移"
                >
                  <Hand size={13} />
                </button>
                <button
                  type="button"
                  className={`hud-icon-btn ${tool === "crop" ? "active" : ""}`}
                  onClick={() => setTool("crop")}
                  title="裁剪图片"
                >
                  <Crop size={13} />
                </button>
                <button
                  type="button"
                  className={`hud-icon-btn ${tool === "mosaic" ? "active" : ""}`}
                  onClick={() => {
                    setCropRect(null);
                    setTool("mosaic");
                  }}
                  title="马赛克打码"
                >
                  <Grid size={13} />
                </button>
                <button
                  type="button"
                  className={`hud-icon-btn ${tool === "rect" ? "active" : ""}`}
                  onClick={() => {
                    setCropRect(null);
                    setTool("rect");
                  }}
                  title="矩形标注"
                >
                  <Square size={13} />
                </button>
                <button
                  type="button"
                  className={`hud-icon-btn ${tool === "ellipse" ? "active" : ""}`}
                  onClick={() => {
                    setCropRect(null);
                    setTool("ellipse");
                  }}
                  title="椭圆标注"
                >
                  <Circle size={13} />
                </button>
                <button
                  type="button"
                  className={`hud-icon-btn ${tool === "arrow" ? "active" : ""}`}
                  onClick={() => {
                    setCropRect(null);
                    setTool("arrow");
                  }}
                  title="箭头标注"
                >
                  <ArrowUpRight size={13} />
                </button>
                <button
                  type="button"
                  className={`hud-icon-btn ${tool === "pen" ? "active" : ""}`}
                  onClick={() => {
                    setCropRect(null);
                    setTool("pen");
                  }}
                  title="画笔标注"
                >
                  <Pencil size={13} />
                </button>
                <button
                  type="button"
                  className={`hud-icon-btn ${tool === "text" ? "active" : ""}`}
                  onClick={() => {
                    setCropRect(null);
                    setTool("text");
                  }}
                  title="文字标注"
                >
                  <Type size={13} />
                </button>
                <button
                  type="button"
                  className="hud-icon-btn"
                  disabled={undoStack.length === 0}
                  onClick={handleUndo}
                  title={`撤销 (${MOD_LABEL}+Z)`}
                >
                  <Undo2 size={13} />
                </button>
                {hasUnsavedEdits && (
                  <button
                    type="button"
                    className="hud-icon-btn"
                    onClick={handleResetEdits}
                    title="还原未保存修改"
                  >
                    <RotateCcw size={13} />
                  </button>
                )}
              </div>

              <div className="stack-preview-actions">
                <button
                  type="button"
                  className="stack-top-pill"
                  onClick={() => switchViewMode("workspace", undefined, "stitch")}
                  title="拼图导出"
                >
                  <Columns2 size={11} /> 拼图
                </button>
                <button
                  type="button"
                  className="stack-top-pill"
                  onClick={() => switchViewMode("workspace", undefined, "translate")}
                  title="批量翻译"
                >
                  <Languages size={11} /> 翻译
                </button>
                <button
                  type="button"
                  className="stack-top-pill"
                  onClick={() => switchViewMode("workspace", undefined, "ocr")}
                  title="批量识字"
                >
                  <ScanText size={11} /> 识字
                </button>
                <button
                  type="button"
                  className="stack-top-pill accent"
                  onClick={() => switchViewMode("workspace", undefined, "ask")}
                  title="多图问 AI"
                >
                  <Sparkles size={11} /> 问 AI
                </button>
              </div>
              <div className="stack-tool-group stack-preview-file-actions">
                <button type="button" className="hud-icon-btn" onClick={handlePreviewScanQr} title="识别二维码" aria-label="识别二维码">
                  <QrCode size={13} />
                </button>
                <button type="button" className="hud-icon-btn" onClick={handlePreviewCopy} title="复制当前截图" aria-label="复制当前截图">
                  {copiedKey === "preview-copy" ? <Check size={13} /> : <Copy size={13} />}
                </button>
                {state.items.length > 1 && (
                  <button type="button" className="hud-icon-btn" onClick={handleCopyAsFiles} title="一键复制全部截图文件（可粘贴到微信/文件夹）" aria-label="复制全部截图">
                    {copiedKey === "files" ? <Check size={13} /> : <Files size={13} />}
                  </button>
                )}
                <button type="button" className="hud-icon-btn" onClick={handlePreviewSaveFile} title="保存 PNG" aria-label="保存 PNG">
                  <Download size={13} />
                </button>
                {hasUnsavedEdits && (
                  <button
                    type="button"
                    className="hud-icon-btn primary"
                    disabled={savingEdits}
                    onClick={saveCurrentEdits}
                    title="保存修改到叠堆"
                  >
                    {savingEdits ? <Loader2 size={12} className="spin" /> : <Check size={12} />} 保存
                  </button>
                )}
              </div>
            </div>
            {qrResults && qrResults.length > 0 && (
              <div className="stack-qr-banner">
                <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", fontWeight: 600 }}>
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
                    <QrCode size={12} /> 识别到 {qrResults.length} 个二维码
                  </span>
                  <button type="button" className="btn btn-sm" onClick={() => setQrResults(null)}>关闭</button>
                </div>
                {qrResults.map((code, i) => {
                  const isUrl = /^https?:\/\//i.test(code.trim());
                  return (
                    <div key={i} className="stack-qr-row">
                      <span className="stack-qr-text">{code}</span>
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() => copyText(code, `qr-${i}`)}
                      >
                        {copiedKey === `qr-${i}` ? <Check size={11} /> : <Copy size={11} />} 复制
                      </button>
                      {isUrl && (
                        <button
                          type="button"
                          className="btn btn-sm"
                          onClick={() => invoke("open_external_url", { url: code }).catch((err) => showNotice(String(err)))}
                        >
                          <ExternalLink size={11} /> 打开
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      </div>
    );
  };

  // ==================== 3. Expanded Stack Floating Drawer + Anchored Corner Deck ====================
  const renderWorkspace = () => (
    <div
      className="stack-expanded-shell"
      onClick={(e) => {
        if (e.target === e.currentTarget) {
          void switchViewMode("collapsed");
        }
      }}
    >
      <div className="stack-workspace" onClick={(e) => e.stopPropagation()}>
        <header className="result-header" data-tauri-drag-region>
          <div className="stack-drawer-title">
            <Layers size={14} style={{ color: "var(--accent)", flexShrink: 0 }} />
            <span className="stack-drawer-title-text">
              {activeTab === "cards"
                ? "叠堆浮层 · 拖起排序 / 点击 × 离场"
                : `叠堆工作台 (${state.items.length}/${state.maxStackSize})`}
            </span>
          </div>

          <div className="icon-actions">
            <button
              type="button"
              className="icon-btn"
              onClick={() => switchViewMode("preview", state.items.length - 1)}
              title="预览与编辑"
            >
              <Eye size={14} />
            </button>
            <button
              type="button"
              className="icon-btn"
              onClick={() => invoke("start_capture", { hideMain: false }).catch(console.error)}
              title="继续截图"
            >
              <Camera size={14} />
            </button>
            {state.items.length > 1 ? (
              <Popconfirm
                message={`清空全部 ${state.items.length} 张截图？`}
                confirmLabel="清空"
                onConfirm={() => {
                  invoke("clear_stack").catch(console.error);
                }}
              >
                {(triggerProps) => (
                  <button
                    type="button"
                    className="icon-btn"
                    title="清空叠堆"
                    {...triggerProps}
                  >
                    <Trash2 size={14} />
                  </button>
                )}
              </Popconfirm>
            ) : (
              <button
                type="button"
                className="icon-btn"
                onClick={() => invoke("clear_stack").catch(console.error)}
                title="清空叠堆"
              >
                <Trash2 size={14} />
              </button>
            )}
            <button
              type="button"
              className="icon-btn"
              onClick={() => switchViewMode("collapsed")}
              title="收起"
            >
              <Minimize2 size={14} />
            </button>
          </div>
        </header>

        {/* Mode tabs */}
        <div className="stack-tabs">
          <SlidingSegmented<StackWorkspaceTab>
            ariaLabel="叠堆工作模式"
            size="sm"
            className="stack-tabs-seg five-col"
            value={activeTab}
            onChange={(next) => {
              if (next === "translate" || next === "ocr") {
                switchBatchTab(next);
              } else {
                setActiveTab(next);
              }
            }}
            options={[
              {
                value: "cards",
                label: (
                  <>
                    <ListOrdered size={12} /> 排序
                  </>
                ),
              },
              {
                value: "stitch",
                label: (
                  <>
                    <Columns2 size={12} /> 拼图
                  </>
                ),
              },
              {
                value: "translate",
                label: (
                  <>
                    <Languages size={12} /> 翻译
                  </>
                ),
              },
              {
                value: "ocr",
                label: (
                  <>
                    <ScanText size={12} /> 识字
                  </>
                ),
              },
              {
                value: "ask",
                label: (
                  <>
                    <Sparkles size={12} /> 问 AI
                  </>
                ),
              },
            ]}
          />
        </div>

        {/* Horizontal thumbnail strip shown on stitch/translate/ocr/ask tabs */}
        {activeTab !== "cards" && (
          <div className="stack-thumb-strip" ref={thumbStripRef}>
            {renderThumbItems()}
          </div>
        )}

        {feedback && !undoToast && <div className="stack-notice">{feedback}</div>}

        <div className={`stack-body${activeTab === "cards" ? " is-cards-tab" : ""}`}>
          {activeTab === "cards" && (
            <div key="cards" className="stack-cards-pane stack-pane-enter">
              <div className="stack-cards-subbar">
                <button
                  type="button"
                  className="stack-exit-style-btn"
                  onClick={toggleDeleteStyle}
                  title="切换点击 × 时的卡片离场动画"
                >
                  <Wand2 size={11} />
                  <span>
                    {deleteStyle === "peel"
                      ? "离场效果：Peel 撕贴纸"
                      : "离场效果：Blocks 弹性消散"}
                  </span>
                </button>
                <div className="stack-cards-subbar-right">
                  <button
                    type="button"
                    className={`stack-mini-pill-btn${copiedKey === "collapsed-copy" ? " copied" : ""}`}
                    onClick={() => void handleCollapsedCopyAll()}
                    title={state.items.length > 1 ? `复制全部 ${state.items.length} 张` : "复制截图"}
                  >
                    {copiedKey === "collapsed-copy" ? <Check size={11} /> : <Copy size={11} />}
                    <span>复制</span>
                  </button>
                </div>
              </div>
              {renderVerticalDragList()}
            </div>
          )}

          {activeTab === "stitch" && (
            <div key="stitch" className="stack-stitch-pane stack-pane-enter">
              <div className="result-section-head">
                <span>拼接方向</span>
                <span>共 {state.items.length} 张</span>
              </div>
              <div className="stack-dir-row">
                <button
                  type="button"
                  className={`stack-dir-btn ${direction === "vertical" ? "active" : ""}`}
                  onClick={() => setDirection("vertical")}
                >
                  <Rows2 size={15} />
                  <span>纵向</span>
                </button>
                <button
                  type="button"
                  className={`stack-dir-btn ${direction === "horizontal" ? "active" : ""}`}
                  onClick={() => setDirection("horizontal")}
                >
                  <Columns2 size={15} />
                  <span>横向</span>
                </button>
                <button
                  type="button"
                  className={`stack-dir-btn ${direction === "grid" ? "active" : ""}`}
                  onClick={() => setDirection("grid")}
                >
                  <Grid size={15} />
                  <span>宫格</span>
                </button>
              </div>

              <div className="stack-stitch-preview">
                <div className="result-section-head">
                  <span>预览</span>
                  <span>
                    {direction === "vertical"
                      ? "纵向顺序"
                      : direction === "horizontal"
                        ? "横向顺序"
                        : "自适应宫格"}
                  </span>
                </div>
                <div className={`stack-stitch-preview-grid ${direction}`}>
                  {state.items.slice(0, 6).map((item) => (
                    <div key={item.id} className="stack-stitch-preview-cell">
                      <img src={item.thumbDataUrl} alt={item.title} draggable={false} />
                    </div>
                  ))}
                </div>
              </div>

              <div className="stack-export-actions">
                <button type="button" className="btn btn-primary" onClick={handleCopyStitched}>
                  {copiedKey === "stitch-copy" ? (
                    <span className="icon-morph">
                      <Check size={14} />
                    </span>
                  ) : (
                    <Copy size={14} />
                  )}
                  复制拼图
                </button>
                <button type="button" className="btn" onClick={handleSaveStitched}>
                  <Download size={14} /> 保存 PNG
                </button>
              </div>

              <button type="button" className="btn" style={{ width: "100%" }} onClick={handleCopyAsFiles}>
                {copiedKey === "files" ? (
                  <span className="icon-morph">
                    <Check size={14} />
                  </span>
                ) : (
                  <Files size={14} />
                )}
                复制为图片文件（支持多图粘贴）
              </button>
            </div>
          )}

          {(activeTab === "translate" || activeTab === "ocr") && (
            <div key={activeTab} className="stack-batch-pane stack-pane-enter">
              {activeTab === "translate" && (
                <div className="stack-batch-controls">
                  <div className="mini-engine-row">
                    {ENGINE_OPTIONS.map((opt) => (
                      <button
                        key={opt.id}
                        type="button"
                        className={`mini-chip ${engine === opt.id ? "active" : ""}`}
                        onClick={() => {
                          setEngine(opt.id);
                          runBatch("translate", opt.id, targetLang);
                        }}
                      >
                        {opt.label}
                      </button>
                    ))}
                  </div>
                  <select
                    className="mini-select"
                    value={targetLang}
                    onChange={(e) => {
                      const lang = e.target.value;
                      setTargetLang(lang);
                      runBatch("translate", engine, lang);
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

              {state.batchMode === activeTab && state.batchLoading && (() => {
                const activeItems =
                  batchStream &&
                  batchStream.jobId === state.batchJobId &&
                  batchStream.kind === activeTab
                    ? batchStream.items
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
                    <div className="loading-status" style={{ padding: "8px 0" }}>
                      <Loader2 size={14} className="spin" />
                      <span>正在处理 {state.items.length} 张截图…</span>
                    </div>
                    {streamedIndices.length > 0 && (
                      <div className="batch-item-list">
                        {streamedIndices.map((idx) => {
                          const entry = activeItems![idx];
                          const title = state.items[idx]
                            ? `图 ${idx + 1} · ${state.items[idx].title}`
                            : `图 ${idx + 1}`;
                          return (
                            <div key={idx} className="batch-item-card">
                              <div className="batch-item-head">
                                <span>{title}</span>
                              </div>
                              {entry.text && (
                                <div className="batch-item-text">
                                  {entry.text}
                                  <span className="stream-cursor" />
                                </div>
                              )}
                              {activeTab === "translate" && entry.originalText && (
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

              {state.batchMode === activeTab && state.batchError && <div className="error-box">{state.batchError}</div>}

              {state.batchMode === activeTab && !state.batchLoading && state.batchItems.length > 0 && (
                <>
                  <div className="result-section-head">
                    <span>
                      已完成 {state.batchItems.length} 项 · {state.batchElapsedMs}ms
                    </span>
                    <div style={{ display: "flex", gap: 6 }}>
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() => runBatch(activeTab)}
                      >
                        重新运行
                      </button>
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() =>
                          copyText(
                            state.batchItems
                              .map(
                                (it) =>
                                  `[${it.title}]\n${
                                    activeTab === "ocr" ? it.originalText : it.translatedText
                                  }`
                              )
                              .join("\n\n"),
                            "batch-all"
                          )
                        }
                      >
                        {copiedKey === "batch-all" ? <Check size={12} /> : <Copy size={12} />}
                        复制全部
                      </button>
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={handleExportBatchMarkdown}
                        title="导出为 Markdown 笔记"
                      >
                        <FileText size={12} /> 导出 MD
                      </button>
                    </div>
                  </div>
                  <div className="batch-item-list">
                    {state.batchItems.map((it) => {
                      const mainText =
                        activeTab === "ocr"
                          ? it.originalText || it.translatedText
                          : it.translatedText;
                      return (
                        <div key={it.index} className="batch-item-card">
                          <div className="batch-item-head">
                            <span>
                              {it.title} · {it.sourceLang}
                            </span>
                            <button
                              type="button"
                              className="icon-btn"
                              onClick={() => copyText(mainText, `batch-${it.index}`)}
                              title="复制此项"
                            >
                              {copiedKey === `batch-${it.index}` ? (
                                <Check size={12} />
                              ) : (
                                <Copy size={12} />
                              )}
                            </button>
                          </div>
                          {activeTab === "translate" && it.blocks && it.blocks.length > 1 ? (
                            <div className="bilingual-pair-list">
                              {it.blocks.map((blk, bIdx) => (
                                <div key={bIdx} className="bilingual-pair-row">
                                  <span className="bilingual-pair-idx">{bIdx + 1}</span>
                                  <div className="bilingual-pair-body">
                                    <div className="bilingual-pair-trans">{blk.translatedText}</div>
                                    {blk.originalText && (
                                      <div className="bilingual-pair-orig">{blk.originalText}</div>
                                    )}
                                  </div>
                                </div>
                              ))}
                            </div>
                          ) : (
                            <div className="batch-item-text">{mainText || "（未检测到文字）"}</div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                </>
              )}
            </div>
          )}

          {activeTab === "ask" && (
            <div key="ask" className="stack-ask-pane stack-pane-enter">
              <div className="ask-thread">
                {state.askMessages.map((msg, idx) => (
                  <div key={idx} className={`ask-bubble ${msg.role}`}>
                    {msg.content}
                  </div>
                ))}
                {state.askLoading &&
                  (askStream && askStream.jobId === state.askJobId && askStream.text ? (
                    <div className="ask-bubble assistant streaming">
                      {askStream.text}
                      <span className="stream-cursor" />
                    </div>
                  ) : (
                    <div className="loading-status">
                      <Loader2 size={13} className="spin" />
                      <span>正在分析 {state.items.length} 张截图…</span>
                    </div>
                  ))}
                {state.askError && <div className="error-box">{state.askError}</div>}
                <div ref={threadEndRef} />
              </div>
            </div>
          )}
        </div>

        {activeTab === "ask" && (
          <>
            <AskQuickActionsBar
              loading={state.askLoading}
              onTrigger={(prompt) => void sendAsk(prompt)}
            />
            <div className="ask-input-dock">
            {state.askMessages.length > 0 && (
              <>
                <button
                  type="button"
                  className="icon-btn"
                  onClick={handleExportAskMarkdown}
                  title="导出对话为 Markdown 笔记"
                >
                  <FileText size={14} />
                </button>
                <button
                  type="button"
                  className="icon-btn"
                  onClick={() => invoke("stack_clear_chat").catch(console.error)}
                  title="清空对话"
                >
                  <Trash2 size={14} />
                </button>
              </>
            )}
            <textarea
              className="ask-textarea"
              rows={1}
              value={askInput}
              onChange={(e) => setAskInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  sendAsk();
                }
              }}
              placeholder={`结合这 ${state.items.length} 张图向 AI 提问...`}
            />
            <button
              type="button"
              className="btn btn-primary btn-sm"
              onClick={() => sendAsk()}
              disabled={state.askLoading || !askInput.trim()}
            >
              <Send size={13} />
            </button>
            </div>
          </>
        )}
      </div>

      <div className="stack-expanded-bottom-row">
        <div className="stack-expanded-toast-slot">
          {renderUndoToast("workspace")}
        </div>
        {renderCornerDeck(true)}
      </div>
    </div>
  );

  const renderView = { collapsed: renderCollapsed, preview: renderPreview, workspace: renderWorkspace };
  return (
    <div className="stack-view-surface">
      {(leavingMode ? [leavingMode, viewMode] : [viewMode]).map((mode) => (
        <div
          key={mode}
          className="stack-mode-layer"
          ref={(element) => {
            modeLayers.current[mode] = element;
            if (element) element.inert = leavingMode !== null;
          }}
          data-stack-mode={mode}
          style={STACK_SIZES[mode]}
        >
          {renderView[mode]()}
        </div>
      ))}
    </div>
  );
}
