// Generate the app source icon (1024x1024 PNG) without any image dependency:
// a Fluent-blue rounded square with a white monitor + play glyph.
// Feed it to `npx tauri icon` to produce the full platform icon set.
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SIZE = 1024;
const SS = 2; // supersampling factor per axis

function clamp01(v) {
  return Math.min(Math.max(v, 0), 1);
}

function lerp(a, b, t) {
  return a + (b - a) * t;
}

function mix(c1, c2, t) {
  return [lerp(c1[0], c2[0], t), lerp(c1[1], c2[1], t), lerp(c1[2], c2[2], t)];
}

// Rounded-rectangle coverage via a signed distance function.
function roundedRectCoverage(x, y, x0, y0, x1, y1, r) {
  const cx = Math.min(Math.max(x, x0 + r), x1 - r);
  const cy = Math.min(Math.max(y, y0 + r), y1 - r);
  const dist = Math.hypot(x - cx, y - cy) - r;
  return clamp01(0.5 - dist / 1.6);
}

// Thick-line coverage (round caps) via distance to segment.
function segmentCoverage(x, y, x0, y0, x1, y1, thickness) {
  const vx = x1 - x0;
  const vy = y1 - y0;
  const wx = x - x0;
  const wy = y - y0;
  const len2 = vx * vx + vy * vy;
  const t = len2 > 0 ? clamp01((wx * vx + wy * vy) / len2) : 0;
  const dx = wx - t * vx;
  const dy = wy - t * vy;
  const dist = Math.hypot(dx, dy) - thickness / 2;
  return clamp01(0.5 - dist / 1.6);
}

// Background gradient: top (0,120,212) -> bottom (0,88,163)
function bgColor(t) {
  return [
    Math.round(lerp(0, 0, t)),
    Math.round(lerp(120, 88, t)),
    Math.round(lerp(212, 163, t)),
  ];
}

const WHITE = [255, 255, 255];
const THICK = 64;

// Monitor + play glyph: screen outline (4 strokes), stand, base and a
// filled play triangle, roughly centered on the canvas.
const SEGMENTS = [
  { x0: 288, y0: 336, x1: 736, y1: 336 }, // screen top
  { x0: 288, y0: 600, x1: 736, y1: 600 }, // screen bottom
  { x0: 288, y0: 336, x1: 288, y1: 600 }, // screen left
  { x0: 736, y0: 336, x1: 736, y1: 600 }, // screen right
  { x0: 512, y0: 600, x1: 512, y1: 712 }, // stand
  { x0: 424, y0: 712, x1: 600, y1: 712 }, // base
];

// Filled triangle coverage via signed edge functions (clockwise winding,
// y-down coordinates: points inside have positive edge distances).
function triangleCoverage(x, y, p0, p1, p2) {
  const edges = [
    [p0, p1],
    [p1, p2],
    [p2, p0],
  ];
  let dist = Infinity;
  for (const [a, b] of edges) {
    const vx = b[0] - a[0];
    const vy = b[1] - a[1];
    const wx = x - a[0];
    const wy = y - a[1];
    const len = Math.hypot(vx, vy);
    const side = (vx * wy - vy * wx) / len; // positive inside
    if (side < dist) dist = side;
  }
  return clamp01(0.5 + dist / 1.6);
}

const PLAY = [
  [462, 398],
  [588, 468],
  [462, 538],
];

function pixel(px, py) {
  // 2x2 supersample
  let r = 0;
  let g = 0;
  let b = 0;
  let a = 0;
  for (let sy = 0; sy < SS; sy++) {
    for (let sx = 0; sx < SS; sx++) {
      const x = px + (sx + 0.5) / SS;
      const y = py + (sy + 0.5) / SS;

      const bg = roundedRectCoverage(x, y, 0, 0, SIZE, SIZE, 176);
      let color = [0, 0, 0];
      if (bg > 0) {
        color = bgColor(clamp01((y / SIZE - 0.12) / 0.76));
        let glyph = 0;
        for (const s of SEGMENTS) {
          glyph = Math.max(glyph, segmentCoverage(x, y, s.x0, s.y0, s.x1, s.y1, THICK));
        }
        glyph = Math.max(glyph, triangleCoverage(x, y, PLAY[0], PLAY[1], PLAY[2]));
        if (glyph > 0) color = mix(color, WHITE, glyph);
      }
      r += color[0] * bg;
      g += color[1] * bg;
      b += color[2] * bg;
      a += bg;
    }
  }
  const n = SS * SS;
  return [Math.round(r / n), Math.round(g / n), Math.round(b / n), Math.round((a / n) * 255)];
}

// ---- PNG encode ------------------------------------------------------

function crc32(buf) {
  let crc = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    let c = (crc ^ buf[i]) & 0xff;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    crc = (crc >>> 8) ^ c;
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const typeBuf = Buffer.from(type, "ascii");
  const crcBuf = Buffer.alloc(4);
  crcBuf.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])));
  return Buffer.concat([len, typeBuf, data, crcBuf]);
}

const raw = Buffer.alloc(SIZE * (SIZE * 4 + 1));
let off = 0;
for (let y = 0; y < SIZE; y++) {
  raw[off++] = 0; // filter: none
  for (let x = 0; x < SIZE; x++) {
    const [pr, pg, pb, pa] = pixel(x, y);
    raw[off++] = pr;
    raw[off++] = pg;
    raw[off++] = pb;
    raw[off++] = pa;
  }
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // color type: RGBA
ihdr[10] = 0; // compression
ihdr[11] = 0; // filter
ihdr[12] = 0; // interlace

const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const outDir = join(root, "scripts");
mkdirSync(outDir, { recursive: true });
const out = join(outDir, "icon.png");
writeFileSync(out, png);
console.log(`[gen-icon] wrote ${out} (${SIZE}x${SIZE})`);
