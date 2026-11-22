import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const iconsDir = path.resolve(__dirname, "..", "src-tauri", "icons");

fs.mkdirSync(iconsDir, { recursive: true });

// Precompute CRC32 table for valid PNG chunks
const crcTable = new Uint32Array(256);
for (let n = 0; n < 256; n++) {
  let c = n;
  for (let k = 0; k < 8; k++) {
    c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  }
  crcTable[n] = c >>> 0;
}

function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    c = crcTable[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  }
  return (c ^ 0xffffffff) >>> 0;
}

function makeChunk(type, data) {
  const typeBuf = Buffer.from(type, "ascii");
  const lenBuf = Buffer.alloc(4);
  lenBuf.writeUInt32BE(data.length, 0);
  const crcInput = Buffer.concat([typeBuf, data]);
  const crcBuf = Buffer.alloc(4);
  crcBuf.writeUInt32BE(crc32(crcInput), 0);
  return Buffer.concat([lenBuf, typeBuf, data, crcBuf]);
}

function renderBrandRgba(size) {
  const buf = Buffer.alloc(size * size * 4);
  const radius = size * 0.22;
  const pad = size * 0.04;

  // Rounded-rect signed distance helper
  function inRoundedRect(x, y, left, top, right, bottom, r) {
    const cx = Math.max(left + r, Math.min(x, right - r));
    const cy = Math.max(top + r, Math.min(y, bottom - r));
    const dx = x - cx;
    const dy = y - cy;
    return Math.hypot(dx, dy) <= r;
  }

  // Draw crisp viewfinder brackets + inner translation bars
  const stroke = Math.max(1.5, size * 0.075);
  const m = size * 0.24;
  const arm = size * 0.18;

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const idx = (y * size + x) * 4;
      const insideBg = inRoundedRect(
        x + 0.5,
        y + 0.5,
        pad,
        pad,
        size - pad,
        size - pad,
        radius
      );

      if (!insideBg) {
        buf[idx] = 0;
        buf[idx + 1] = 0;
        buf[idx + 2] = 0;
        buf[idx + 3] = 0;
        continue;
      }

      // Indigo -> Sky gradient background
      const t = (x + y) / (2 * size);
      let r = Math.round(37 + (14 - 37) * t); // #2563eb -> #0ea5e9
      let g = Math.round(99 + (165 - 99) * t);
      let b = Math.round(235 + (233 - 235) * t);

      const px = x + 0.5;
      const py = y + 0.5;
      const left = m;
      const right = size - m;
      const top = m;
      const bottom = size - m;

      // Viewfinder corners
      const topLeftH = py >= top && py <= top + stroke && px >= left && px <= left + arm;
      const topLeftV = px >= left && px <= left + stroke && py >= top && py <= top + arm;
      const topRightH = py >= top && py <= top + stroke && px >= right - arm && px <= right;
      const topRightV = px >= right - stroke && px <= right && py >= top && py <= top + arm;
      const botLeftH = py >= bottom - stroke && py <= bottom && px >= left && px <= left + arm;
      const botLeftV = px >= left && px <= left + stroke && py >= bottom - arm && py <= bottom;
      const botRightH = py >= bottom - stroke && py <= bottom && px >= right - arm && px <= right;
      const botRightV = px >= right - stroke && px <= right && py >= bottom - arm && py <= bottom;

      // Center translation lines (two clean horizontal bars)
      const bar1 =
        py >= size * 0.42 &&
        py <= size * 0.42 + stroke * 0.85 &&
        px >= size * 0.35 &&
        px <= size * 0.65;
      const bar2 =
        py >= size * 0.56 &&
        py <= size * 0.56 + stroke * 0.85 &&
        px >= size * 0.35 &&
        px <= size * 0.56;

      if (
        topLeftH ||
        topLeftV ||
        topRightH ||
        topRightV ||
        botLeftH ||
        botLeftV ||
        botRightH ||
        botRightV ||
        bar1 ||
        bar2
      ) {
        r = 255;
        g = 255;
        b = 255;
      }

      buf[idx] = r;
      buf[idx + 1] = g;
      buf[idx + 2] = b;
      buf[idx + 3] = 255;
    }
  }
  return buf;
}

function encodePng(size, rgba) {
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;

  const stride = size * 4;
  const raw = Buffer.alloc((stride + 1) * size);
  for (let y = 0; y < size; y++) {
    raw[y * (stride + 1)] = 0; // filter type 0
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }
  const compressed = zlib.deflateSync(raw);

  return Buffer.concat([
    signature,
    makeChunk("IHDR", ihdr),
    makeChunk("IDAT", compressed),
    makeChunk("IEND", Buffer.alloc(0)),
  ]);
}

// Wrap a PNG inside a standard Windows ICO container (supported since Windows Vista)
function encodeIcoFromPng(size, pngBuffer) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type 1 = ICO
  header.writeUInt16LE(1, 4); // count = 1

  const entry = Buffer.alloc(16);
  entry[0] = size >= 256 ? 0 : size;
  entry[1] = size >= 256 ? 0 : size;
  entry[2] = 0; // palette colors
  entry[3] = 0; // reserved
  entry.writeUInt16LE(1, 4); // color planes
  entry.writeUInt16LE(32, 6); // bpp
  entry.writeUInt32LE(pngBuffer.length, 8);
  entry.writeUInt32LE(22, 12); // image offset = 6 + 16

  return Buffer.concat([header, entry, pngBuffer]);
}

// Package PNG assets into a standard macOS ICNS container (supported since macOS 10.7).
function encodeIcnsFromPngs(entries) {
  const chunks = entries.map(({ type, pngBuffer }) => {
    const chunk = Buffer.alloc(8 + pngBuffer.length);
    chunk.write(type, 0, 4, "ascii");
    chunk.writeUInt32BE(8 + pngBuffer.length, 4);
    pngBuffer.copy(chunk, 8);
    return chunk;
  });
  const totalLength = 8 + chunks.reduce((sum, c) => sum + c.length, 0);
  const header = Buffer.alloc(8);
  header.write("icns", 0, 4, "ascii");
  header.writeUInt32BE(totalLength, 4);
  return Buffer.concat([header, ...chunks]);
}

const targets = [
  { name: "32x32.png", size: 32 },
  { name: "128x128.png", size: 128 },
  { name: "128x128@2x.png", size: 256 },
  { name: "icon.png", size: 256 },
];

for (const t of targets) {
  const outPath = path.join(iconsDir, t.name);
  if (!fs.existsSync(outPath)) {
    const rgba = renderBrandRgba(t.size);
    fs.writeFileSync(outPath, encodePng(t.size, rgba));
  }
}

const icoPath = path.join(iconsDir, "icon.ico");
if (!fs.existsSync(icoPath)) {
  const rgba256 = renderBrandRgba(256);
  const png256 = encodePng(256, rgba256);
  fs.writeFileSync(icoPath, encodeIcoFromPng(256, png256));
}

const icnsPath = path.join(iconsDir, "icon.icns");
if (!fs.existsSync(icnsPath)) {
  const icnsSpecs = [
    { type: "ic07", size: 128 },
    { type: "ic08", size: 256 },
    { type: "ic09", size: 512 },
    { type: "ic10", size: 1024 },
  ];
  const icnsEntries = icnsSpecs.map(({ type, size }) => ({
    type,
    pngBuffer: encodePng(size, renderBrandRgba(size)),
  }));
  fs.writeFileSync(icnsPath, encodeIcnsFromPngs(icnsEntries));
}

console.log("[SnapTrans] Icons ready in src-tauri/icons");
