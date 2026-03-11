import { useEffect, useState } from "react";

export const PRODUCT_WORD = "Borg";
const LETTERS = "BORG".split("");

// Unicode ranges for glitch characters — [start, end] inclusive
// Only standalone visible glyphs with near-universal font support
const GLYPH_RANGES: [number, number][] = [
  [0x0021, 0x007e], // ASCII printable
  [0x00c0, 0x024f], // Latin Extended
  [0x0391, 0x03c9], // Greek letters
  [0x0400, 0x04ff], // Cyrillic
  [0x05d0, 0x05ea], // Hebrew letters (no combining marks)
  [0x0621, 0x064a], // Arabic letters (no harakat/control chars)
  [0x0904, 0x0939], // Devanagari vowels + consonants
  [0x0e01, 0x0e2e], // Thai consonants
  [0x10d0, 0x10fa], // Georgian Mkhedruli
  [0x3040, 0x309f], // Hiragana
  [0x30a0, 0x30ff], // Katakana
  [0x4e00, 0x9fff], // CJK Unified
  [0xac00, 0xd7af], // Hangul syllables
];

function randomGlyph(): string {
  const range = GLYPH_RANGES[Math.floor(Math.random() * GLYPH_RANGES.length)];
  const cp = range[0] + Math.floor(Math.random() * (range[1] - range[0] + 1));
  return String.fromCodePoint(cp);
}

// Groups of indices that can shift together
const GROUPS = [
  [0], // B alone
  [1], // O alone
  [2], // R alone
  [3], // G alone
  [0, 1], // top row
  [2, 3], // bottom row
  [0, 2], // left col
  [1, 3], // right col
  [0, 1, 2], // three
  [1, 2, 3], // three
  [0, 1, 2, 3], // all four
];

function randomShift(): { x: number; y: number } {
  const px = () => (Math.random() < 0.5 ? -1 : 1) * (1 + Math.random() * 2.5);
  // Pick x, y, or both
  const r = Math.random();
  if (r < 0.4) return { x: px(), y: 0 };
  if (r < 0.8) return { x: 0, y: px() };
  return { x: px(), y: px() };
}

type Offsets = { x: number; y: number }[];

function BorgLogo({ size = "desktop", expanded }: { size?: "desktop" | "mobile"; expanded?: boolean }) {
  const textSize = size === "desktop" ? "text-[22px]" : "text-[16px]";
  const [cells, setCells] = useState(LETTERS);
  const [offsets, setOffsets] = useState<Offsets>(() => LETTERS.map(() => ({ x: 0, y: 0 })));
  // Letter flicker — picks 1-4 letters per tick with decreasing probability
  useEffect(() => {
    const timers = new Set<ReturnType<typeof setTimeout>>();
    let stopped = false;

    function scheduleFlicker() {
      if (stopped) return;
      const delay = 4800 + Math.random() * 14400;
      const timer = setTimeout(() => {
        timers.delete(timer);
        const r = Math.random();
        const count = r < 0.6 ? 1 : r < 0.9 ? 2 : r < 0.98 ? 3 : 4;
        const indices = [0, 1, 2, 3].sort(() => Math.random() - 0.5).slice(0, count);

        setCells((prev) => {
          const next = [...prev];
          for (const idx of indices) {
            next[idx] = randomGlyph();
          }
          return next;
        });

        const restore = setTimeout(
          () => {
            timers.delete(restore);
            setCells((prev) => {
              const next = [...prev];
              for (const idx of indices) {
                next[idx] = LETTERS[idx];
              }
              return next;
            });
          },
          60 + Math.random() * 940,
        );
        timers.add(restore);
        scheduleFlicker();
      }, delay);
      timers.add(timer);
    }

    scheduleFlicker();
    return () => {
      stopped = true;
      timers.forEach(clearTimeout);
    };
  }, []);

  // Positional shifts
  useEffect(() => {
    const timers = new Set<ReturnType<typeof setTimeout>>();
    let stopped = false;

    function scheduleShift() {
      if (stopped) return;
      const delay = 3000 + Math.random() * 8000;
      const t = setTimeout(() => {
        timers.delete(t);
        const group = GROUPS[Math.floor(Math.random() * GROUPS.length)];
        const shift = randomShift();

        setOffsets((prev) => {
          const next = [...prev];
          for (const idx of group) {
            next[idx] = { x: shift.x, y: shift.y };
          }
          return next;
        });

        const restore = setTimeout(
          () => {
            timers.delete(restore);
            setOffsets((prev) => {
              const next = [...prev];
              for (const idx of group) {
                next[idx] = { x: 0, y: 0 };
              }
              return next;
            });
          },
          30 + Math.random() * 20,
        );
        timers.add(restore);
        scheduleShift();
      }, delay);
      timers.add(t);
    }

    scheduleShift();
    return () => {
      stopped = true;
      timers.forEach(clearTimeout);
    };
  }, []);

  return (
    <div
      className={`borg-logo-text grid h-full w-full grid-cols-2 grid-rows-2 ${expanded ? "group-hover/nav:grid-cols-4 group-hover/nav:grid-rows-1" : ""}`}
    >
      {cells.map((c, i) => (
        <span
          key={i}
          className={`flex items-center justify-center ${textSize} text-white`}
          style={{
            transform: `translate(${offsets[i].x}px, ${offsets[i].y}px)`,
            transition: "transform 80ms steps(1, end)",
          }}
        >
          {c}
        </span>
      ))}
    </div>
  );
}

export { BorgLogo };
