import { TranslatedBlock } from "../types";

export const INPLACE_FONT_FAMILY = '"Segoe UI", "Microsoft YaHei", sans-serif';
export const INPLACE_FONT_WEIGHT = 400;

const THEME_SAMPLE_SIZE = 24;
const CJK_RE = /[぀-ヿ㐀-鿿가-힯]/;
const LATIN_ASC_RE = /[A-Z0-9bdfhklt()[\]{}|/\\!?#$%&]/;
const LATIN_DESC_RE = /[gjpqy()[\]{}|]/;
const NO_LINE_START_PUNCT_RE = /^[。，、！？；：）》」』】….,!?;:)\]]$/;

export interface SampledTheme {
  bgColor: string;
  textColor: string;
  bgR?: number;
  bgG?: number;
  bgB?: number;
  borderMad?: number;
}

export interface ComputedBlockLayout {
  index: number;
  block: TranslatedBlock;
  unchanged?: boolean;
  patchX: number;
  patchY: number;
  patchW: number;
  patchH: number;
  textOffsetX: number;
  textOffsetY: number;
  maxTextW: number;
  fontSize: number;
  lineHeight: number;
  lines: string[];
  bgColor: string;
  textColor: string;
}

let cachedMeasureCtx: CanvasRenderingContext2D | null = null;
let cachedStripCanvas: HTMLCanvasElement | null = null;
let cachedStripCtx: CanvasRenderingContext2D | null = null;

function getMeasureContext(): CanvasRenderingContext2D | null {
  if (cachedMeasureCtx) return cachedMeasureCtx;
  if (typeof document === "undefined") return null;
  const c = document.createElement("canvas");
  c.width = 16;
  c.height = 16;
  cachedMeasureCtx = c.getContext("2d");
  return cachedMeasureCtx;
}

function getStripSampleContext(
  w: number,
  h: number
): CanvasRenderingContext2D | null {
  if (typeof document === "undefined") return null;
  if (!cachedStripCanvas) {
    cachedStripCanvas = document.createElement("canvas");
    cachedStripCtx = cachedStripCanvas.getContext("2d", {
      willReadFrequently: true,
    });
  }
  if (!cachedStripCtx) return null;
  if (cachedStripCanvas.width !== w) cachedStripCanvas.width = w;
  if (cachedStripCanvas.height !== h) cachedStripCanvas.height = h;
  return cachedStripCtx;
}

export function createThemeSampleContext(): CanvasRenderingContext2D | null {
  if (typeof document === "undefined") return null;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = THEME_SAMPLE_SIZE;
  return canvas.getContext("2d", { willReadFrequently: true });
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

function normalizeCompareText(s: string): string {
  return s.replace(/\s+/g, " ").trim().toLowerCase();
}

export function isUnchangedTranslation(
  originalText: string | undefined,
  translatedText: string
): boolean {
  const orig = normalizeCompareText(originalText || "");
  const trans = normalizeCompareText(translatedText || "");
  if (!orig || !trans) return false;
  return orig === trans;
}

export function wrapCanvasText(
  ctx: CanvasRenderingContext2D,
  text: string,
  maxWidth: number
): string[] {
  const safeMaxW = Math.max(8, maxWidth);
  const lines: string[] = [];
  for (const para of text.split(/\r?\n/)) {
    if (!para) {
      lines.push("");
      continue;
    }
    let current = "";
    for (const ch of para) {
      const candidate = current + ch;
      if (ctx.measureText(candidate).width > safeMaxW && current.length > 0) {
        if (NO_LINE_START_PUNCT_RE.test(ch) && current.length > 1) {
          lines.push(current.slice(0, -1));
          current = current.slice(-1) + ch;
        } else {
          lines.push(current);
          current = ch === " " ? "" : ch;
        }
      } else {
        current = candidate;
      }
    }
    if (current) lines.push(current);
  }
  return lines.length > 0 ? lines : [""];
}

function medianChannel(values: number[]): number {
  if (values.length === 0) return 248;
  const sorted = values.slice().sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)];
}

/**
 * Sample the dominant background color along the outer border of a rectangle
 * using per-channel median so adjacent glyphs/icons don't tint the mask.
 */
export function sampleOpaqueBlockTheme(
  sampleCtx: CanvasRenderingContext2D | null,
  image: HTMLImageElement,
  sx: number,
  sy: number,
  sw: number,
  sh: number
): SampledTheme {
  if (!sampleCtx || !image.naturalWidth || !image.naturalHeight) {
    return {
      bgColor: "rgb(15, 23, 42)",
      textColor: "#f8fafc",
      bgR: 15,
      bgG: 23,
      bgB: 42,
      borderMad: 99,
    };
  }
  const margin = 2;
  const x0 = Math.max(0, Math.floor(sx - margin));
  const y0 = Math.max(0, Math.floor(sy - margin));
  const x1 = Math.min(image.naturalWidth, Math.ceil(sx + sw + margin));
  const y1 = Math.min(image.naturalHeight, Math.ceil(sy + sh + margin));
  const cw = Math.max(1, x1 - x0);
  const ch = Math.max(1, y1 - y0);

  const size = THEME_SAMPLE_SIZE;
  sampleCtx.clearRect(0, 0, size, size);
  sampleCtx.drawImage(image, x0, y0, cw, ch, 0, 0, size, size);
  const pixels = sampleCtx.getImageData(0, 0, size, size).data;

  const rs: number[] = [];
  const gs: number[] = [];
  const bs: number[] = [];
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      if (x !== 0 && x !== size - 1 && y !== 0 && y !== size - 1) continue;
      const idx = (y * size + x) * 4;
      rs.push(pixels[idx]);
      gs.push(pixels[idx + 1]);
      bs.push(pixels[idx + 2]);
    }
  }

  const r = medianChannel(rs);
  const g = medianChannel(gs);
  const b = medianChannel(bs);

  let madSum = 0;
  for (let i = 0; i < rs.length; i++) {
    madSum +=
      (Math.abs(rs[i] - r) + Math.abs(gs[i] - g) + Math.abs(bs[i] - b)) / 3;
  }
  const borderMad = rs.length > 0 ? madSum / rs.length : 99;

  const luma = 0.299 * r + 0.587 * g + 0.114 * b;
  return {
    bgColor: `rgb(${r}, ${g}, ${b})`,
    textColor: luma > 145 ? "#18181b" : "#f8fafc",
    bgR: r,
    bgG: g,
    bgB: b,
    borderMad,
  };
}

interface VerticalPixelAnalysis {
  topSplitY: number | null;
  bottomSplitY: number | null;
  inkTop: number | null;
  inkBottom: number | null;
}

interface NeighborSpan {
  top: number;
  bottom: number;
}

function findCleanValley(
  rowInk: Float32Array,
  rStart: number,
  rEnd: number,
  preferredRow: number,
  minAllowedRow: number,
  maxAllowedRow: number
): { start: number; end: number } | null {
  const lo = Math.max(0, Math.min(rowInk.length - 1, Math.floor(rStart)));
  const hi = Math.max(lo, Math.min(rowInk.length - 1, Math.ceil(rEnd)));
  if (hi - lo < 1) return null;

  const runs: Array<{ start: number; end: number }> = [];
  let curStart = -1;
  for (let r = lo; r <= hi; r++) {
    if (rowInk[r] < 0.008) {
      if (curStart < 0) curStart = r;
    } else if (curStart >= 0) {
      runs.push({ start: curStart, end: r - 1 });
      curStart = -1;
    }
  }
  if (curStart >= 0) {
    runs.push({ start: curStart, end: hi });
  }

  let best: { start: number; end: number } | null = null;
  let bestScore = -Infinity;
  for (const run of runs) {
    const mid = (run.start + run.end) * 0.5;
    if (mid < minAllowedRow || mid > maxAllowedRow) continue;
    const len = run.end - run.start + 1;
    const dist = Math.abs(mid - preferredRow);
    const score = len * 2.2 - dist * 0.4;
    if (score > bestScore) {
      bestScore = score;
      best = run;
    }
  }
  if (best) return best;

  // Fallback: single minimum-ink row when anti-aliased strokes barely kiss.
  let minRow = -1;
  let minInk = 0.06;
  for (let r = lo; r <= hi; r++) {
    if (r < minAllowedRow || r > maxAllowedRow) continue;
    if (rowInk[r] < minInk) {
      minInk = rowInk[r];
      minRow = r;
    }
  }
  return minRow >= 0 ? { start: minRow, end: minRow } : null;
}

/**
 * Inspect actual screenshot rows around a block to find:
 * 1. Exact clean background valleys separating tightly-spaced vertical neighbors.
 * 2. Tight single-line glyph ink bounds (excluding OCR detection box bloat).
 */
function analyzeBlockVerticalPixels(
  image: HTMLImageElement | null,
  theme: SampledTheme,
  regionSx: number,
  regionSy: number,
  regionSw: number,
  regionSh: number,
  boxW: number,
  boxH: number,
  origLeft: number,
  origTop: number,
  origW: number,
  origH: number,
  origLineH: number,
  isSingleLine: boolean,
  prevNeighbor: NeighborSpan | null,
  nextNeighbor: NeighborSpan | null,
  scale: number
): VerticalPixelAnalysis {
  const empty: VerticalPixelAnalysis = {
    topSplitY: null,
    bottomSplitY: null,
    inkTop: null,
    inkBottom: null,
  };
  if (
    !image ||
    !image.naturalWidth ||
    !image.naturalHeight ||
    (theme.borderMad ?? 99) >= 18 ||
    origW < 4 ||
    origH < 4
  ) {
    return empty;
  }

  const bgR = theme.bgR ?? 255;
  const bgG = theme.bgG ?? 255;
  const bgB = theme.bgB ?? 255;
  const origBottom = origTop + origH;
  const closeThreshold = Math.max(8 * scale, origLineH * 0.6);
  const hasClosePrev =
    prevNeighbor !== null && origTop - prevNeighbor.bottom < closeThreshold;
  const hasCloseNext =
    nextNeighbor !== null && nextNeighbor.top - origBottom < closeThreshold;

  const scanTopBox = Math.max(
    0,
    hasClosePrev && prevNeighbor
      ? Math.min(origTop - 2 * scale, (prevNeighbor.top + prevNeighbor.bottom) * 0.5)
      : origTop - 2 * scale
  );
  const scanBottomBox = Math.min(
    boxH,
    hasCloseNext && nextNeighbor
      ? Math.max(
          origBottom + 2 * scale,
          (nextNeighbor.top + nextNeighbor.bottom) * 0.5
        )
      : origBottom + 2 * scale
  );
  const scanHBox = scanBottomBox - scanTopBox;
  if (scanHBox <= 2) return empty;

  const sx0 = clamp(
    Math.floor(regionSx + (origLeft / Math.max(1, boxW)) * regionSw),
    0,
    image.naturalWidth - 1
  );
  const sx1 = clamp(
    Math.ceil(regionSx + ((origLeft + origW) / Math.max(1, boxW)) * regionSw),
    sx0 + 1,
    image.naturalWidth
  );
  const sy0 = clamp(
    Math.floor(regionSy + (scanTopBox / Math.max(1, boxH)) * regionSh),
    0,
    image.naturalHeight - 1
  );
  const sy1 = clamp(
    Math.ceil(regionSy + (scanBottomBox / Math.max(1, boxH)) * regionSh),
    sy0 + 1,
    image.naturalHeight
  );

  const srcW = sx1 - sx0;
  const srcH = sy1 - sy0;
  const sampleW = clamp(Math.round(srcW), 16, 256);
  const sampleH = clamp(Math.round(srcH), 8, 96);
  const stripCtx = getStripSampleContext(sampleW, sampleH);
  if (!stripCtx) return empty;

  stripCtx.clearRect(0, 0, sampleW, sampleH);
  stripCtx.drawImage(image, sx0, sy0, srcW, srcH, 0, 0, sampleW, sampleH);
  const data = stripCtx.getImageData(0, 0, sampleW, sampleH).data;

  const rowInk = new Float32Array(sampleH);
  for (let y = 0; y < sampleH; y++) {
    let inkCount = 0;
    const rowOff = y * sampleW * 4;
    for (let x = 0; x < sampleW; x++) {
      const i = rowOff + x * 4;
      const diff =
        Math.abs(data[i] - bgR) +
        Math.abs(data[i + 1] - bgG) +
        Math.abs(data[i + 2] - bgB);
      if (diff > 56) inkCount++;
    }
    rowInk[y] = inkCount / sampleW;
  }

  const boxYToRow = (y: number) => ((y - scanTopBox) / scanHBox) * sampleH;
  const rowEdgeToBoxY = (r: number) => scanTopBox + (r / sampleH) * scanHBox;

  let topSplitY: number | null = null;
  let bottomSplitY: number | null = null;
  let topValleyBottomY: number | null = null;
  let bottomValleyTopY: number | null = null;

  if (hasClosePrev && prevNeighbor) {
    const prevMidY = (prevNeighbor.top + prevNeighbor.bottom) * 0.5;
    const bMidY = origTop + origH * 0.5;
    const preferredY = (prevNeighbor.bottom + origTop) * 0.5;
    const valley = findCleanValley(
      rowInk,
      boxYToRow(prevMidY),
      boxYToRow(bMidY),
      boxYToRow(preferredY),
      boxYToRow(prevNeighbor.top + (prevNeighbor.bottom - prevNeighbor.top) * 0.55),
      boxYToRow(origTop + origH * 0.45)
    );
    if (valley) {
      const vTopY = rowEdgeToBoxY(valley.start);
      const vBotY = rowEdgeToBoxY(valley.end + 1);
      topValleyBottomY = vBotY;
      const vMidY = (vTopY + vBotY) * 0.5;
      topSplitY = Math.min(
        vBotY,
        Math.max(vTopY + 0.75 * scale, vMidY)
      );
    }
  }

  if (hasCloseNext && nextNeighbor) {
    const bMidY = origTop + origH * 0.5;
    const nextMidY = (nextNeighbor.top + nextNeighbor.bottom) * 0.5;
    const preferredY = (origBottom + nextNeighbor.top) * 0.5;
    const valley = findCleanValley(
      rowInk,
      boxYToRow(bMidY),
      boxYToRow(nextMidY),
      boxYToRow(preferredY),
      boxYToRow(origTop + origH * 0.55),
      boxYToRow(nextNeighbor.top + (nextNeighbor.bottom - nextNeighbor.top) * 0.45)
    );
    if (valley) {
      const vTopY = rowEdgeToBoxY(valley.start);
      const vBotY = rowEdgeToBoxY(valley.end + 1);
      bottomValleyTopY = vTopY;
      const vMidY = (vTopY + vBotY) * 0.5;
      bottomSplitY = Math.max(
        vTopY,
        Math.min(vBotY - 0.75 * scale, vMidY)
      );
    }
  }

  let inkTop: number | null = null;
  let inkBottom: number | null = null;
  if (isSingleLine) {
    const boundTopY = topValleyBottomY ?? origTop;
    const boundBottomY = bottomValleyTopY ?? origBottom;
    const rLo = clamp(Math.floor(boxYToRow(boundTopY)), 0, sampleH - 1);
    const rHi = clamp(Math.ceil(boxYToRow(boundBottomY)) - 1, rLo, sampleH - 1);

    let firstInk = -1;
    let lastInk = -1;
    for (let r = rLo; r <= rHi; r++) {
      if (rowInk[r] >= 0.008) {
        if (firstInk < 0) firstInk = r;
        lastInk = r;
      }
    }
    if (firstInk >= 0 && lastInk >= firstInk) {
      const candTop = rowEdgeToBoxY(firstInk);
      const candBot = rowEdgeToBoxY(lastInk + 1);
      const candH = candBot - candTop;
      if (candH >= Math.max(5 * scale, origH * 0.32) && candH <= origH * 1.08) {
        inkTop = candTop;
        inkBottom = candBot;
      }
    }
  }

  return { topSplitY, bottomSplitY, inkTop, inkBottom };
}

function estimateStartFontSize(
  originalText: string | undefined,
  translatedText: string,
  origLineH: number,
  inkH: number | null,
  scale: number
): { startFont: number; isCjkTarget: boolean } {
  const isCjkTarget = CJK_RE.test(translatedText);
  let estOrigFont: number;

  if (inkH !== null && inkH > 0) {
    const orig = (originalText || "").trim();
    let ratio = 0.78;
    if (CJK_RE.test(orig)) {
      ratio = 0.86;
    } else {
      const hasAsc = LATIN_ASC_RE.test(orig);
      const hasDesc = LATIN_DESC_RE.test(orig);
      if (hasAsc && hasDesc) ratio = 0.92;
      else if (hasAsc) ratio = 0.74;
      else if (hasDesc) ratio = 0.72;
      else if (orig.length > 0) ratio = 0.55;
    }
    estOrigFont = clamp(inkH / ratio, origLineH * 0.54, origLineH * 0.84);
  } else {
    // OCR detection boxes (especially RapidOCR DBNet) already include ~25-35% vertical padding.
    estOrigFont = origLineH * 0.74;
  }

  // CJK glyphs fill ~88% of their em-box vs ~55-72% for Latin, so scale CJK down
  // optically to match surrounding text cleanly and keep translated labels compact.
  const opticalScale = isCjkTarget ? 0.82 : 0.90;
  const startFont = clamp(estOrigFont * opticalScale, 9 * scale, 24 * scale);
  return { startFont, isCjkTarget };
}

export function computeInPlaceBlockLayouts(
  blocks: TranslatedBlock[],
  boxW: number,
  boxH: number,
  image: HTMLImageElement | null,
  regionSx: number,
  regionSy: number,
  regionSw: number,
  regionSh: number,
  scale = 1,
  sharedSampleCtx?: CanvasRenderingContext2D | null,
  sharedMeasureCtx?: CanvasRenderingContext2D | null
): ComputedBlockLayout[] {
  if (!blocks || blocks.length === 0 || boxW <= 4 || boxH <= 4) return [];
  const measureCtx = sharedMeasureCtx ?? getMeasureContext();
  const sampleCtx = sharedSampleCtx ?? createThemeSampleContext();
  if (!measureCtx) return [];

  const indexed = blocks
    .map((block, index) => ({ block, index }))
    .filter(({ block }) => Boolean(block.translatedText?.trim()));

  // Process top-to-bottom (then left-to-right) so lower blocks always respect
  // the actual placed patch bottom of upper blocks.
  const ordered = indexed.slice().sort((a, b) => {
    const dy = a.block.y - b.block.y;
    if (Math.abs(dy) > 0.008) return dy;
    return a.block.x - b.block.x;
  });

  const placedRects: Array<{
    left: number;
    top: number;
    right: number;
    bottom: number;
    unchanged: boolean;
    verifiedInk: boolean;
  }> = [];

  const results: ComputedBlockLayout[] = [];
  const minGap = Math.max(1, Math.round(1.5 * scale));

  for (const { block: b, index } of ordered) {
    const text = b.translatedText.trim();
    const origLeft = b.x * boxW;
    const origTop = b.y * boxH;
    const origW = Math.max(8 * scale, b.w * boxW);
    const origH = Math.max(8 * scale, b.h * boxH);
    const origBottom = origTop + origH;
    const rawLineH =
      b.lineHeight > 0
        ? b.lineHeight * boxH
        : origH / Math.max(1, b.lineCount || 1);
    const origLineH = Math.max(10 * scale, rawLineH);
    const isSingleLine = (b.lineCount || 1) <= 1;

    // Sample background theme first (also used to inspect pixel valleys & ink bounds).
    const theme = image
      ? sampleOpaqueBlockTheme(
          sampleCtx,
          image,
          regionSx + b.x * regionSw,
          regionSy + b.y * regionSh,
          Math.max(4, b.w * regionSw),
          Math.max(4, b.h * regionSh)
        )
      : {
          bgColor: "rgb(255, 255, 255)",
          textColor: "#18181b",
          bgR: 255,
          bgG: 255,
          bgB: 255,
          borderMad: 99,
        };

    // Find nearest direct vertical neighbors overlapping this block's horizontal span.
    let prevNeighbor: NeighborSpan | null = null;
    let nextNeighbor: NeighborSpan | null = null;
    const bMidY = b.y + b.h * 0.5;
    for (const { block: o, index: oIdx } of indexed) {
      if (oIdx === index) continue;
      const hOverlap = o.x < b.x + b.w + 0.015 && o.x + o.w > b.x - 0.015;
      if (!hOverlap) continue;
      const oMidY = o.y + o.h * 0.5;
      if (oMidY < bMidY && o.y < b.y + b.h * 0.25) {
        const candBot = (o.y + o.h) * boxH;
        if (!prevNeighbor || candBot > prevNeighbor.bottom) {
          prevNeighbor = { top: o.y * boxH, bottom: candBot };
        }
      } else if (oMidY > bMidY && o.y + o.h > b.y + b.h * 0.75) {
        const candTop = o.y * boxH;
        if (!nextNeighbor || candTop < nextNeighbor.top) {
          nextNeighbor = { top: candTop, bottom: (o.y + o.h) * boxH };
        }
      }
    }

    const pix = analyzeBlockVerticalPixels(
      image,
      theme,
      regionSx,
      regionSy,
      regionSw,
      regionSh,
      boxW,
      boxH,
      origLeft,
      origTop,
      origW,
      origH,
      origLineH,
      isSingleLine,
      prevNeighbor,
      nextNeighbor,
      scale
    );

    const effTop = pix.inkTop ?? origTop;
    const effBottom = pix.inkBottom ?? origBottom;
    const effH = Math.max(6 * scale, effBottom - effTop);

    // Keep unchanged blocks (brand names, benchmarks, numbers, code tokens) as crisp
    // untouched original pixels while still registering them as obstacles for neighbors.
    if (isUnchangedTranslation(b.originalText, text)) {
      placedRects.push({
        left: origLeft,
        top: effTop,
        right: origLeft + origW,
        bottom: effBottom,
        unchanged: true,
        verifiedInk: pix.inkBottom !== null,
      });
      results.push({
        index,
        block: b,
        unchanged: true,
        patchX: origLeft,
        patchY: effTop,
        patchW: origW,
        patchH: effH,
        textOffsetX: 0,
        textOffsetY: 0,
        maxTextW: origW,
        fontSize: Math.max(8.5 * scale, effH),
        lineHeight: effH,
        lines: [],
        bgColor: "transparent",
        textColor: theme.textColor,
      });
      continue;
    }

    // Lean padding: cover original glyphs completely without encroaching on neighbors.
    const padLeft = clamp(origLineH * 0.10, 1 * scale, 2.5 * scale);
    const padRight = clamp(origLineH * 0.12, 1.5 * scale, 3 * scale);
    const padTop =
      pix.inkTop !== null
        ? clamp(effH * 0.14, 1 * scale, 2 * scale)
        : clamp(origLineH * 0.06, 0.5 * scale, 1.5 * scale);
    const padBottom =
      pix.inkBottom !== null
        ? clamp(effH * 0.16, 1 * scale, 2 * scale)
        : clamp(origLineH * 0.08, 1 * scale, 2 * scale);

    // Find nearest neighbor to the right on the same row.
    let availRight = boxW - Math.max(4 * scale, boxW * 0.015);
    for (const { block: o, index: oIdx } of indexed) {
      if (oIdx === index) continue;
      const vOverlap = Math.min(b.y + b.h, o.y + o.h) - Math.max(b.y, o.y);
      if (
        vOverlap > Math.min(b.h, o.h) * 0.25 &&
        o.x >= b.x + b.w * 0.5 &&
        o.x >= b.x + b.w - 0.02
      ) {
        availRight = Math.min(availRight, o.x * boxW - 3.5 * scale);
      }
    }
    availRight = Math.max(origLeft + origW, availRight);

    // Check previously placed blocks directly above so our mask never paints over them.
    let prevPlacedPatchBottom = 0;
    let prevPlacedUnchangedBottom = 0;
    for (const pr of placedRects) {
      const hOverlap = pr.left < origLeft + origW + 2 * scale && pr.right > origLeft - 2 * scale;
      if (!hOverlap) continue;
      const prMidY = (pr.top + pr.bottom) * 0.5;
      if (prMidY < origTop + origH * 0.5) {
        if (pr.unchanged) {
          if (pr.verifiedInk || pr.bottom <= origTop) {
            prevPlacedUnchangedBottom = Math.max(prevPlacedUnchangedBottom, pr.bottom);
          }
        } else {
          prevPlacedPatchBottom = Math.max(prevPlacedPatchBottom, pr.bottom);
        }
      }
    }

    // Hard top limit: never expand upward into an adjacent upper line, even if
    // raw OCR boxes touched or slightly overlapped (prevBottom >= origTop).
    let hardTop = 0;
    if (pix.topSplitY !== null) {
      hardTop = pix.topSplitY;
    } else if (prevNeighbor) {
      if (prevNeighbor.bottom <= origTop) {
        hardTop = prevNeighbor.bottom + minGap;
      } else {
        hardTop = Math.min(
          origTop + origH * 0.28,
          (prevNeighbor.bottom + origTop) * 0.5
        );
      }
    }
    if (prevPlacedUnchangedBottom > 0) {
      hardTop = Math.max(hardTop, prevPlacedUnchangedBottom + 0.75 * scale);
    }
    if (prevPlacedPatchBottom > 0) {
      hardTop = Math.max(hardTop, prevPlacedPatchBottom + 0.5 * scale);
    }

    // Hard bottom limit: stop cleanly above the next line below.
    let hardBottom = boxH;
    if (pix.bottomSplitY !== null) {
      hardBottom = pix.bottomSplitY;
    } else if (nextNeighbor) {
      if (nextNeighbor.top >= origBottom) {
        hardBottom = nextNeighbor.top - minGap;
      } else {
        hardBottom = Math.max(
          origBottom - origH * 0.28,
          (origBottom + nextNeighbor.top) * 0.5
        );
      }
    }

    const maskTop = clamp(
      Math.max(hardTop, effTop - padTop),
      0,
      Math.max(0, effBottom - 4 * scale)
    );
    const minMaskBottom = Math.min(
      hardBottom,
      Math.max(effBottom + padBottom, maskTop + Math.min(effH, 6 * scale))
    );
    const maskBottom = clamp(minMaskBottom, maskTop + 4 * scale, boxH);
    const availBottom = Math.max(maskBottom, Math.min(boxH - 1, hardBottom));
    const availH = Math.max(maskBottom - maskTop, availBottom - maskTop);

    const { startFont, isCjkTarget } = estimateStartFontSize(
      b.originalText,
      text,
      origLineH,
      pix.inkTop !== null && pix.inkBottom !== null
        ? pix.inkBottom - pix.inkTop
        : null,
      scale
    );
    const singleLineRatio = isCjkTarget ? 1.16 : 1.18;
    const multiLineRatio = isCjkTarget ? 1.22 : 1.24;
    const minFont = Math.max(7.5 * scale, startFont * 0.62);
    const step = Math.max(0.5, 0.5 * scale);

    const fitText = (targetMaxW: number) => {
      let bestFont = startFont;
      let bestLineH = startFont * singleLineRatio;
      let bestLines: string[] = [text];
      for (let f = startFont; f >= minFont - 0.01; f -= step) {
        const candidateFont = Math.max(minFont, f);
        measureCtx.font = `${INPLACE_FONT_WEIGHT} ${candidateFont}px ${INPLACE_FONT_FAMILY}`;
        const candidateLines = wrapCanvasText(measureCtx, text, targetMaxW);
        const ratio =
          candidateLines.length <= 1 ? singleLineRatio : multiLineRatio;
        const candidateLineH = candidateFont * ratio;
        bestFont = candidateFont;
        bestLineH = candidateLineH;
        bestLines = candidateLines;
        if (candidateLines.length * candidateLineH <= availH) {
          break;
        }
      }
      measureCtx.font = `${INPLACE_FONT_WEIGHT} ${bestFont}px ${INPLACE_FONT_FAMILY}`;
      let usedW = 0;
      for (const line of bestLines) {
        usedW = Math.max(usedW, measureCtx.measureText(line).width);
      }
      return { fontSize: bestFont, lineHeight: bestLineH, lines: bestLines, usedW };
    };

    let maxTextW = Math.max(16 * scale, availRight - origLeft);
    let fitted = fitText(maxTextW);

    // Guard against horizontal expansion colliding with a vertically-overlapping right column.
    let desiredPatchBottom = Math.min(
      availBottom,
      Math.max(maskBottom, maskTop + fitted.lines.length * fitted.lineHeight + 0.5 * scale)
    );
    if (origLeft + fitted.usedW + padRight > origLeft + origW + padRight) {
      let tightenedRight = availRight;
      for (const { block: o, index: oIdx } of indexed) {
        if (oIdx === index) continue;
        const oLeft = o.x * boxW;
        const oTop = o.y * boxH;
        const oBot = (o.y + o.h) * boxH;
        if (
          oLeft >= origLeft + origW * 0.5 &&
          oTop < desiredPatchBottom &&
          oBot > maskTop
        ) {
          tightenedRight = Math.min(tightenedRight, oLeft - 3.5 * scale);
        }
      }
      if (tightenedRight < availRight - 1 && tightenedRight > origLeft + 16 * scale) {
        availRight = tightenedRight;
        maxTextW = Math.max(16 * scale, availRight - origLeft);
        fitted = fitText(maxTextW);
        desiredPatchBottom = Math.min(
          availBottom,
          Math.max(maskBottom, maskTop + fitted.lines.length * fitted.lineHeight + 0.5 * scale)
        );
      }
    }

    const { fontSize, lineHeight, lines, usedW: usedTextW } = fitted;

    const patchX = Math.max(0, origLeft - padLeft);
    const patchRight = Math.min(
      boxW,
      Math.min(
        availRight + padRight,
        Math.max(origLeft + origW + padRight, origLeft + usedTextW + padRight)
      )
    );
    const patchW = Math.max(8 * scale, patchRight - patchX);

    const usedTextH = lines.length * lineHeight;
    const patchY = maskTop;
    const patchBottom = Math.min(
      availBottom,
      Math.max(maskBottom, Math.min(availBottom, maskTop + usedTextH + 0.5 * scale))
    );
    const patchH = Math.max(maskBottom - maskTop, patchBottom - patchY);

    const textOffsetX = Math.max(1, origLeft - patchX);
    let textOffsetY: number;
    if (lines.length === 1 && isSingleLine) {
      const idealTop = effTop + (effH - lineHeight) / 2;
      textOffsetY = Math.max(
        0,
        Math.min(Math.max(0, patchH - lineHeight), idealTop - patchY)
      );
    } else {
      textOffsetY = Math.max(0, effTop - patchY);
    }

    placedRects.push({
      left: patchX,
      top: patchY,
      right: patchX + patchW,
      bottom: patchY + patchH,
      unchanged: false,
      verifiedInk: pix.inkBottom !== null,
    });

    results.push({
      index,
      block: b,
      unchanged: false,
      patchX,
      patchY,
      patchW,
      patchH,
      textOffsetX,
      textOffsetY,
      maxTextW: Math.max(8, patchW - textOffsetX - 1),
      fontSize,
      lineHeight,
      lines,
      bgColor: theme.bgColor,
      textColor: theme.textColor,
    });
  }

  results.sort((a, b) => a.index - b.index);
  return results;
}
