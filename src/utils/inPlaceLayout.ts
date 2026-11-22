import { TranslatedBlock } from "../types";

const THEME_SAMPLE_SIZE = 24;

export interface SampledTheme {
  bgColor: string;
  textColor: string;
}

export interface ComputedBlockLayout {
  index: number;
  block: TranslatedBlock;
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

function getMeasureContext(): CanvasRenderingContext2D | null {
  if (cachedMeasureCtx) return cachedMeasureCtx;
  if (typeof document === "undefined") return null;
  const c = document.createElement("canvas");
  c.width = 16;
  c.height = 16;
  cachedMeasureCtx = c.getContext("2d");
  return cachedMeasureCtx;
}

export function createThemeSampleContext(): CanvasRenderingContext2D | null {
  if (typeof document === "undefined") return null;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = THEME_SAMPLE_SIZE;
  return canvas.getContext("2d", { willReadFrequently: true });
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
        lines.push(current);
        current = ch === " " ? "" : ch;
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
  values.sort((a, b) => a - b);
  return values[Math.floor(values.length / 2)];
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
    return { bgColor: "rgb(15, 23, 42)", textColor: "#f8fafc" };
  }
  const margin = 3;
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
  const luma = 0.299 * r + 0.587 * g + 0.114 * b;
  return {
    bgColor: `rgb(${r}, ${g}, ${b})`,
    textColor: luma > 145 ? "#18181b" : "#f8fafc",
  };
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

  const results: ComputedBlockLayout[] = [];

  for (const { block: b, index } of indexed) {
    const text = b.translatedText.trim();
    const origLeft = b.x * boxW;
    const origTop = b.y * boxH;
    const origW = Math.max(8 * scale, b.w * boxW);
    const origH = Math.max(8 * scale, b.h * boxH);
    const rawLineH =
      b.lineHeight > 0 ? b.lineHeight * boxH : origH / Math.max(1, b.lineCount || 1);
    const origLineH = Math.max(10 * scale, rawLineH);

    // Keep left padding tight so leading checklist icons/bullets remain untouched.
    const padLeft = Math.min(Math.round(3 * scale), Math.max(1, Math.round(origLineH * 0.14)));
    const padRight = Math.max(2, Math.round(origLineH * 0.18));
    const padTop = Math.max(1, Math.round(origLineH * 0.12));
    const padBottom = Math.max(2, Math.round(origLineH * 0.16));

    // Find nearest neighbor to the right on the same row.
    let availRight = boxW - Math.max(4 * scale, boxW * 0.015);
    for (const { block: o, index: oIdx } of indexed) {
      if (oIdx === index) continue;
      const vOverlap = Math.min(b.y + b.h, o.y + o.h) - Math.max(b.y, o.y);
      if (vOverlap > Math.min(b.h, o.h) * 0.35 && o.x >= b.x + b.w - 0.01) {
        availRight = Math.min(availRight, o.x * boxW - 4 * scale);
      }
    }
    availRight = Math.max(origLeft + origW, availRight);
    const availRightNorm = availRight / Math.max(1, boxW);

    // Find nearest neighbors directly above and below that overlap horizontally.
    let prevBottom = 0;
    let nextTop = boxH - 2 * scale;
    for (const { block: o, index: oIdx } of indexed) {
      if (oIdx === index) continue;
      const hOverlap = o.x < availRightNorm && o.x + o.w > b.x - 0.02;
      if (!hOverlap) continue;
      if (o.y + o.h <= b.y + b.h * 0.5) {
        prevBottom = Math.max(prevBottom, (o.y + o.h) * boxH);
      } else if (o.y >= b.y + b.h * 0.5) {
        nextTop = Math.min(nextTop, o.y * boxH);
      }
    }

    const minGap = Math.max(1, Math.round(1.5 * scale));
    const maskTop =
      prevBottom > 0 && prevBottom < origTop
        ? Math.max(prevBottom + minGap, origTop - padTop)
        : Math.max(0, origTop - padTop);
    const maskBottom = Math.min(
      boxH,
      Math.max(origTop + origH, Math.min(origTop + origH + padBottom, nextTop - minGap))
    );
    const availBottom = Math.max(maskBottom, Math.min(boxH - 1, nextTop - minGap));
    const availH = Math.max(origH, availBottom - maskTop);

    const maxTextW = Math.max(16 * scale, availRight - origLeft);
    const startFont = Math.max(9 * scale, Math.min(28 * scale, origLineH * 0.84));
    const minFont = Math.max(7.5 * scale, startFont * 0.56);
    const step = Math.max(0.5, 0.5 * scale);

    let fontSize = startFont;
    let lineHeight = fontSize * 1.28;
    let lines: string[] = [text];

    for (let f = startFont; f >= minFont - 0.01; f -= step) {
      const candidateFont = Math.max(minFont, f);
      measureCtx.font = `500 ${candidateFont}px "Segoe UI", "Microsoft YaHei", sans-serif`;
      const candidateLineH = candidateFont * 1.28;
      const candidateLines = wrapCanvasText(measureCtx, text, maxTextW);
      fontSize = candidateFont;
      lineHeight = candidateLineH;
      lines = candidateLines;
      if (candidateLines.length * candidateLineH <= availH + 1.5 * scale) {
        break;
      }
    }

    measureCtx.font = `500 ${fontSize}px "Segoe UI", "Microsoft YaHei", sans-serif`;
    let usedTextW = 0;
    for (const line of lines) {
      usedTextW = Math.max(usedTextW, measureCtx.measureText(line).width);
    }

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
      Math.max(maskBottom, origTop + usedTextH + padBottom * 0.5)
    );
    const patchH = Math.max(maskBottom - maskTop, patchBottom - patchY);

    const textOffsetX = Math.max(1, origLeft - patchX);
    let textOffsetY: number;
    if (lines.length === 1 && (b.lineCount || 1) <= 1) {
      const idealTop = origTop + (origH - lineHeight) / 2;
      textOffsetY = Math.max(0, Math.min(Math.max(0, patchH - lineHeight), idealTop - patchY));
    } else {
      textOffsetY = Math.max(0, origTop - patchY);
    }

    const theme = image
      ? sampleOpaqueBlockTheme(
          sampleCtx,
          image,
          regionSx + b.x * regionSw,
          regionSy + b.y * regionSh,
          Math.max(4, b.w * regionSw),
          Math.max(4, b.h * regionSh)
        )
      : { bgColor: "rgb(255, 255, 255)", textColor: "#18181b" };

    results.push({
      index,
      block: b,
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

  return results;
}
