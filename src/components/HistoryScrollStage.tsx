import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  ArrowUp,
  Check,
  ChevronLeft,
  ChevronRight,
  Copy,
  ExternalLink,
  Layers,
  Pause,
  Play,
  Sparkles,
  Trash2,
} from "lucide-react";
import { TranslationResult } from "../types";

export type HistoryScrollMode = "stacked" | "coverflow" | "deck";
export type HistoryViewMode = "list" | HistoryScrollMode;

interface HistoryScrollStageProps {
  items: TranslationResult[];
  mode: HistoryScrollMode;
  onModeChange: (mode: HistoryScrollMode) => void;
  copiedId: number | null;
  exitingIds: Record<number, true>;
  onReopen: (id: number) => void;
  onCopy: (item: TranslationResult) => void;
  onDelete: (id: number) => void;
  formatTimestamp: (ms: number) => string;
  isDemo?: boolean;
  onExitDemo?: () => void;
}

const SCROLL_MODES: readonly {
  id: HistoryScrollMode;
  shortLabel: string;
  title: string;
  desc: string;
}[] = [
  {
    id: "stacked",
    shortLabel: "层叠",
    title: "1. 层叠卡片 Stacked",
    desc: "新记录逐层盖上，下层随滚动实时缩小变暗并留出顶部索引边。",
  },
  {
    id: "coverflow",
    shortLabel: "3D 轮播",
    title: "2. 3D 封面轮播 Cover Flow",
    desc: "居中记录放大聚焦，两侧随滚动实时 3D 偏转与压暗。",
  },
  {
    id: "deck",
    shortLabel: "卡组飞出",
    title: "3. 卡组抛物飞出 Card Deck",
    desc: "顶部卡片沿真实抛物线交替向左右飞出，下方卡片依次升起。",
  },
];

const ENGINE_LABELS: Record<TranslationResult["engine"], string> = {
  ai: "AI 翻译",
  google: "Google",
  deepl: "DeepL",
};

const STEP_PX = 340;
const WINDOW_RADIUS = 6;

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

export function HistoryScrollStage({
  items,
  mode,
  onModeChange,
  copiedId,
  exitingIds,
  onReopen,
  onCopy,
  onDelete,
  formatTimestamp,
  isDemo = false,
  onExitDemo,
}: HistoryScrollStageProps) {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const cardRefs = useRef<Map<number, HTMLElement>>(new Map());
  const pctRef = useRef<HTMLElement | null>(null);
  const fillRef = useRef<HTMLDivElement | null>(null);
  const idxRef = useRef<HTMLSpanElement | null>(null);
  const scrollRafRef = useRef<number | null>(null);
  const autoScrollRafRef = useRef<number | null>(null);
  const activeIdxRef = useRef<number>(0);

  const [activeIdx, setActiveIdx] = useState(0);
  const [autoScrolling, setAutoScrolling] = useState(false);
  const [vpSize, setVpSize] = useState<{ width: number; height: number }>({
    width: 460,
    height: 440,
  });

  const n = items.length;
  const runwayHeight =
    n <= 1 ? Math.max(320, vpSize.height) : vpSize.height + (n - 1) * STEP_PX;

  const stopAutoScroll = useCallback(() => {
    if (autoScrollRafRef.current !== null) {
      window.cancelAnimationFrame(autoScrollRafRef.current);
      autoScrollRafRef.current = null;
    }
    setAutoScrolling(false);
  }, []);

  // Measure viewport dimensions for sticky height and responsive 3D Cover Flow spacing
  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp) return;
    const measure = () => {
      const w = Math.max(260, Math.round(vp.clientWidth));
      const h = Math.max(320, Math.round(vp.clientHeight));
      setVpSize((prev) => (prev.width === w && prev.height === h ? prev : { width: w, height: h }));
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(vp);
    return () => ro.disconnect();
  }, []);

  // Cleanup animations on unmount
  useEffect(() => {
    return () => {
      if (scrollRafRef.current !== null) {
        window.cancelAnimationFrame(scrollRafRef.current);
      }
      if (autoScrollRafRef.current !== null) {
        window.cancelAnimationFrame(autoScrollRafRef.current);
      }
    };
  }, []);

  // Clamp active index if filtered list shrinks
  useEffect(() => {
    if (n === 0) {
      activeIdxRef.current = 0;
      setActiveIdx(0);
      return;
    }
    if (activeIdxRef.current > n - 1) {
      activeIdxRef.current = n - 1;
      setActiveIdx(n - 1);
    }
  }, [n]);

  const applyTransforms = useCallback(() => {
    const vp = viewportRef.current;
    if (!vp) return;

    const count = items.length;
    const maxScroll = Math.max(1, vp.scrollHeight - vp.clientHeight);
    const rawP = count <= 1 ? 0 : Math.max(0, Math.min(1, vp.scrollTop / maxScroll));
    const pos = count <= 1 ? 0 : rawP * (count - 1);
    const nearest = count === 0 ? 0 : Math.max(0, Math.min(count - 1, Math.round(pos)));

    if (pctRef.current) {
      pctRef.current.textContent = `${Math.round(rawP * 100)}%`;
    }
    if (fillRef.current) {
      fillRef.current.style.width = `${(rawP * 100).toFixed(1)}%`;
    }
    if (idxRef.current) {
      idxRef.current.textContent =
        count === 0 ? "00 / 00" : `${pad2(nearest + 1)} / ${pad2(count)}`;
    }

    if (nearest !== activeIdxRef.current) {
      activeIdxRef.current = nearest;
      setActiveIdx(nearest);
    }

    const vpW = Math.max(260, vp.clientWidth);
    const spreadX = Math.max(148, Math.min(232, vpW * 0.37));
    const flyX = Math.max(300, Math.min(440, vpW * 0.86));

    cardRefs.current.forEach((card, i) => {
      const diff = i - pos; // 0 = active, <0 = passed, >0 = upcoming
      const isActive = Math.abs(diff) < 0.36;
      card.dataset.active = isActive ? "true" : "false";

      if (mode === "stacked") {
        if (diff > 0) {
          const enterY = Math.min(440, diff * 375);
          const op = diff > 1.25 ? 0 : Math.max(0, 1 - Math.max(0, diff - 0.8) * 2);
          card.style.transform = `translate3d(0, ${enterY.toFixed(1)}px, 0) scale(1)`;
          card.style.opacity = op.toFixed(3);
          card.style.filter = "none";
          card.style.zIndex = String(10 + i);
          card.style.pointerEvents = diff < 0.35 && op > 0.08 ? "auto" : "none";
        } else {
          const depth = Math.min(3.6, -diff);
          const scale = Math.max(0.84, 1 - depth * 0.048);
          const y = -depth * 18;
          const dim = Math.max(0.58, 1 - depth * 0.14);
          const op = depth > 3.25 ? Math.max(0, 1 - (depth - 3.25) * 3) : 1;
          card.style.transform = `translate3d(0, ${y.toFixed(1)}px, 0) scale(${scale.toFixed(4)})`;
          card.style.opacity = op.toFixed(3);
          card.style.filter = depth > 0.01 ? `brightness(${dim.toFixed(3)})` : "none";
          card.style.zIndex = String(10 + i);
          card.style.pointerEvents = op > 0.08 ? "auto" : "none";
        }
      } else if (mode === "coverflow") {
        const clamped = Math.max(-2.6, Math.min(2.6, diff));
        const absC = Math.abs(clamped);
        const x = clamped * spreadX;
        const rotY = Math.max(-42, Math.min(42, -clamped * 28));
        const z = -absC * 145;
        const scale = Math.max(0.76, 1 - absC * 0.12);
        const dim = Math.max(0.56, 1 - absC * 0.19);
        const op = Math.max(0, 1 - Math.max(0, absC - 1.55) * 0.92);
        card.style.transform = `translate3d(${x.toFixed(1)}px, 0, ${z.toFixed(1)}px) rotateY(${rotY.toFixed(1)}deg) scale(${scale.toFixed(3)})`;
        card.style.opacity = op.toFixed(3);
        card.style.filter = absC > 0.01 ? `brightness(${dim.toFixed(3)})` : "none";
        card.style.zIndex = String(Math.round(50 - absC * 10));
        card.style.pointerEvents = op > 0.08 ? "auto" : "none";
      } else {
        // Card Deck: top card flies out alternately left/right along a real parabola; below rises smoothly
        if (diff < 0) {
          const t = Math.min(1.2, -diff);
          const dir = i % 2 === 0 ? -1 : 1;
          const x = dir * t * flyX;
          const y = -48 * Math.sin(Math.min(1, t) * Math.PI) + t * t * 110;
          const rot = dir * t * 22;
          const op = Math.max(0, 1 - t * 1.15);
          card.style.transform = `translate3d(${x.toFixed(1)}px, ${y.toFixed(1)}px, 0) rotate(${rot.toFixed(1)}deg)`;
          card.style.opacity = op.toFixed(3);
          card.style.filter = "none";
          card.style.zIndex = String(1000 - i);
          card.style.pointerEvents = t < 0.22 && op > 0.2 ? "auto" : "none";
        } else {
          const depth = Math.min(3.6, diff);
          const scale = Math.max(0.84, 1 - depth * 0.048);
          const y = depth * 18;
          const dim = Math.max(0.66, 1 - depth * 0.11);
          const op = depth > 3.15 ? Math.max(0, 1 - (depth - 3.15) * 2.6) : 1;
          card.style.transform = `translate3d(0, ${y.toFixed(1)}px, 0) scale(${scale.toFixed(4)})`;
          card.style.opacity = op.toFixed(3);
          card.style.filter = depth > 0.01 ? `brightness(${dim.toFixed(3)})` : "none";
          card.style.zIndex = String(1000 - i);
          card.style.pointerEvents = depth < 0.35 && op > 0.08 ? "auto" : "none";
        }
      }
    });
  }, [items.length, mode]);

  // Synchronously position newly rendered cards before browser paint
  useLayoutEffect(() => {
    applyTransforms();
  }, [applyTransforms, activeIdx, items, vpSize]);

  const handleScroll = useCallback(() => {
    if (scrollRafRef.current !== null) return;
    scrollRafRef.current = window.requestAnimationFrame(() => {
      scrollRafRef.current = null;
      applyTransforms();
    });
  }, [applyTransforms]);

  const scrollToIndex = useCallback(
    (targetIndex: number) => {
      stopAutoScroll();
      const vp = viewportRef.current;
      if (!vp || n <= 1) return;
      const clamped = Math.max(0, Math.min(n - 1, targetIndex));
      const maxScroll = Math.max(0, vp.scrollHeight - vp.clientHeight);
      const top = (clamped / (n - 1)) * maxScroll;
      vp.scrollTo({ top, behavior: "smooth" });
    },
    [n, stopAutoScroll]
  );

  const toggleAutoScroll = useCallback(() => {
    if (autoScrolling) {
      stopAutoScroll();
      return;
    }
    const vp = viewportRef.current;
    if (!vp || n <= 1) return;
    const maxScroll = vp.scrollHeight - vp.clientHeight;
    if (maxScroll <= 4) return;

    const startTop = vp.scrollTop >= maxScroll - 16 ? 0 : vp.scrollTop;
    vp.scrollTop = startTop;
    const remainingRatio = Math.max(0.15, (maxScroll - startTop) / maxScroll);
    const totalDur = Math.max(2600, Math.min(9000, (n - 1) * 920));
    const dur = totalDur * remainingRatio;
    const t0 = performance.now();

    setAutoScrolling(true);
    const step = (now: number) => {
      const p = Math.min(1, (now - t0) / dur);
      const eased = p < 0.5 ? 2 * p * p : 1 - Math.pow(-2 * p + 2, 2) / 2;
      vp.scrollTop = startTop + (maxScroll - startTop) * eased;
      applyTransforms();
      if (p < 1) {
        autoScrollRafRef.current = window.requestAnimationFrame(step);
      } else {
        autoScrollRafRef.current = null;
        setAutoScrolling(false);
      }
    };
    autoScrollRafRef.current = window.requestAnimationFrame(step);
  }, [applyTransforms, autoScrolling, n, stopAutoScroll]);

  // Stop auto-demo cleanly when items list changes significantly
  useEffect(() => {
    stopAutoScroll();
  }, [n, stopAutoScroll]);

  const winStart = Math.max(0, activeIdx - WINDOW_RADIUS);
  const winEnd = Math.min(n - 1, activeIdx + WINDOW_RADIUS);
  const visibleSlice = n > 0 ? items.slice(winStart, winEnd + 1) : [];

  return (
    <div className="hscroll-shell">
      {/* Left Mode Selector & Real-Time Scroll Progress Rail */}
      <aside className="hscroll-sidebar" aria-label="滚动驱动模式">
        <div className="hscroll-rail-label">
          <Layers size={12} />
          <span>滚动驱动模式</span>
        </div>

        <div className="hscroll-mode-list" role="tablist" aria-label="卡片交互模式">
          {SCROLL_MODES.map((m) => {
            const active = mode === m.id;
            return (
              <button
                key={m.id}
                type="button"
                role="tab"
                aria-selected={active}
                className={`hscroll-mode-card${active ? " active" : ""}`}
                onClick={() => onModeChange(m.id)}
              >
                <strong className="hscroll-mode-title">{m.title}</strong>
                <span className="hscroll-mode-short">{m.shortLabel}</span>
                <p className="hscroll-mode-desc">{m.desc}</p>
              </button>
            );
          })}
        </div>

        <div className="hscroll-meter">
          <div className="hscroll-meter-head">
            <span>实时滚动进度</span>
            <strong ref={pctRef} className="hscroll-mono">
              0%
            </strong>
          </div>
          <div className="hscroll-meter-track">
            <div ref={fillRef} className="hscroll-meter-fill" />
          </div>
          <p className="hscroll-meter-note">
            滚轮控制权 100% 归你，停在一半画面就停在一半，回滚即丝滑倒放。
          </p>
        </div>
      </aside>

      {/* Right Native-Scroll Stage (Zero Wheel Hijacking) */}
      <div
        ref={viewportRef}
        className="hscroll-viewport"
        tabIndex={0}
        aria-label="历史卡片滚动舞台"
        onScroll={handleScroll}
        onWheel={stopAutoScroll}
        onPointerDown={stopAutoScroll}
        onTouchStart={stopAutoScroll}
        onKeyDown={(e) => {
          if (n <= 1) return;
          if (e.key === "ArrowDown" || e.key === "ArrowRight") {
            e.preventDefault();
            scrollToIndex(activeIdxRef.current + 1);
          } else if (e.key === "ArrowUp" || e.key === "ArrowLeft") {
            e.preventDefault();
            scrollToIndex(activeIdxRef.current - 1);
          } else if (e.key === "Home") {
            e.preventDefault();
            scrollToIndex(0);
          } else if (e.key === "End") {
            e.preventDefault();
            scrollToIndex(n - 1);
          }
        }}
      >
        <div className="hscroll-runway" style={{ height: `${runwayHeight}px` }}>
          <div
            className={`hscroll-sticky mode-${mode}`}
            style={{ height: `${vpSize.height}px` }}
          >
            {/* Top HUD */}
            <div className="hscroll-hud-top">
              <div className="hscroll-hud-left">
                {isDemo ? (
                  <span className="hscroll-pill is-demo">
                    <Sparkles size={11} /> 示例卡片演示
                  </span>
                ) : (
                  <span className="hscroll-pill">
                    {n > 1 ? "↕ 上下滚动滚轮" : "≥ 2 条记录可用滚轮切换"}
                  </span>
                )}
                {isDemo && onExitDemo && (
                  <button
                    type="button"
                    className="btn btn-sm hscroll-mini-btn"
                    onClick={onExitDemo}
                  >
                    退出示例
                  </button>
                )}
              </div>

              <div className="hscroll-hud-right">
                {n > 1 && (
                  <>
                    <button
                      type="button"
                      className={`btn btn-sm hscroll-mini-btn${autoScrolling ? " is-playing" : ""}`}
                      onClick={toggleAutoScroll}
                      title={autoScrolling ? "停止自动演示" : "自动慢滚演示"}
                    >
                      {autoScrolling ? <Pause size={11} /> : <Play size={11} />}
                      <span>{autoScrolling ? "停止" : "慢滚"}</span>
                    </button>
                    <button
                      type="button"
                      className="btn btn-sm hscroll-mini-btn"
                      onClick={() => scrollToIndex(0)}
                      title="回到顶部"
                    >
                      <ArrowUp size={11} />
                      <span>回顶</span>
                    </button>
                    <div className="hscroll-step-nav">
                      <button
                        type="button"
                        className="icon-btn hscroll-nav-btn"
                        disabled={activeIdx <= 0}
                        onClick={() => scrollToIndex(activeIdx - 1)}
                        title="上一条"
                        aria-label="上一条"
                      >
                        <ChevronLeft size={13} />
                      </button>
                      <button
                        type="button"
                        className="icon-btn hscroll-nav-btn"
                        disabled={activeIdx >= n - 1}
                        onClick={() => scrollToIndex(activeIdx + 1)}
                        title="下一条"
                        aria-label="下一条"
                      >
                        <ChevronRight size={13} />
                      </button>
                    </div>
                  </>
                )}
                <span ref={idxRef} className="hscroll-idx hscroll-mono">
                  {n === 0 ? "00 / 00" : `${pad2(activeIdx + 1)} / ${pad2(n)}`}
                </span>
              </div>
            </div>

            {/* 3D / Stacked / Deck Card Stage */}
            <div className={`hscroll-stage mode-${mode}`}>
              {visibleSlice.map((item, offset) => {
                const i = winStart + offset;
                const ts = formatTimestamp(item.createdAtMs);
                const isExiting = Boolean(exitingIds[item.id]);
                const hasOrig = Boolean(item.originalText && item.mode !== "ocr");

                return (
                  <article
                    key={item.id}
                    ref={(el) => {
                      if (el) {
                        cardRefs.current.set(i, el);
                      } else {
                        cardRefs.current.delete(i);
                      }
                    }}
                    className={`hscroll-card${isExiting ? " exiting" : ""}`}
                    data-active={i === activeIdx ? "true" : "false"}
                    onClick={(e) => {
                      const el = cardRefs.current.get(i);
                      if (el && el.dataset.active !== "true") {
                        e.stopPropagation();
                        scrollToIndex(i);
                      }
                    }}
                  >
                    <div className="hscroll-card-top">
                      <div className="hscroll-card-top-left">
                        <span className="hscroll-card-no hscroll-mono">NO.{pad2(i + 1)}</span>
                        <span className="v3-mini-badge">
                          {item.mode === "ocr"
                            ? "识字"
                            : `${item.sourceLang} → ${item.targetLang}`}
                        </span>
                      </div>
                      <span className="hscroll-card-meta">
                        {item.model} · {item.elapsedMs}ms{ts ? ` · ${ts}` : ""}
                      </span>
                    </div>

                    <div className="hscroll-card-body">
                      <div className="hscroll-card-main-row">
                        {item.imageDataUrl && (
                          <img
                            src={item.imageDataUrl}
                            alt=""
                            className="hscroll-card-thumb"
                            draggable={false}
                          />
                        )}
                        <div
                          className={`hscroll-card-text${hasOrig ? " has-orig" : ""}`}
                          title={item.translatedText}
                        >
                          {item.translatedText}
                        </div>
                      </div>

                      {hasOrig && (
                        <div className="hscroll-card-src" title={item.originalText}>
                          {item.originalText}
                        </div>
                      )}
                    </div>

                    <div className="hscroll-card-foot">
                      <span className="hscroll-card-engine">
                        {isDemo ? "示例数据" : ENGINE_LABELS[item.engine] ?? item.engine}
                      </span>
                      <div className="hscroll-card-actions">
                        {!isDemo && (
                          <button
                            type="button"
                            className="btn btn-sm"
                            onClick={(e) => {
                              e.stopPropagation();
                              onReopen(item.id);
                            }}
                            title="在浮窗中重新打开"
                          >
                            <ExternalLink size={12} /> 浮窗
                          </button>
                        )}
                        <button
                          type="button"
                          className="btn btn-sm"
                          onClick={(e) => {
                            e.stopPropagation();
                            onCopy(item);
                          }}
                        >
                          {copiedId === item.id ? (
                            <>
                              <span className="icon-morph">
                                <Check size={12} />
                              </span>
                              已复制
                            </>
                          ) : (
                            <>
                              <Copy size={12} /> 复制
                            </>
                          )}
                        </button>
                        {!isDemo && (
                          <button
                            type="button"
                            className="icon-btn history-del-btn"
                            onClick={(e) => {
                              e.stopPropagation();
                              onDelete(item.id);
                            }}
                            title="删除"
                            aria-label="删除此条记录"
                          >
                            <Trash2 size={13} />
                          </button>
                        )}
                      </div>
                    </div>
                  </article>
                );
              })}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
