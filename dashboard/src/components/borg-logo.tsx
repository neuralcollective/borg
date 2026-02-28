import { useState, useEffect, useRef } from "react";

const WORD = "BORG";
const LETTERS = WORD.split("");

// Groups of indices that can shift together
const GROUPS = [
  [0],          // B alone
  [1],          // O alone
  [2],          // R alone
  [3],          // G alone
  [0, 1],       // top row
  [2, 3],       // bottom row
  [0, 2],       // left col
  [1, 3],       // right col
  [0, 1, 2],    // three
  [1, 2, 3],    // three
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

function BorgLogo({ size = "desktop" }: { size?: "desktop" | "mobile" }) {
  const textSize = size === "desktop" ? "text-[22px]" : "text-[16px]";
  const [cells, setCells] = useState(LETTERS);
  const [offsets, setOffsets] = useState<Offsets>(() =>
    LETTERS.map(() => ({ x: 0, y: 0 }))
  );
  const timers = useRef<ReturnType<typeof setTimeout>[]>([]);

  // Letter flicker — picks 1-4 letters per tick with decreasing probability
  useEffect(() => {
    function scheduleFlicker() {
      const delay = 6000 + Math.random() * 18000;
      const timer = setTimeout(() => {
        // Weighted pick: 1 letter 60%, 2 30%, 3 8%, 4 2%
        const r = Math.random();
        const count = r < 0.6 ? 1 : r < 0.9 ? 2 : r < 0.98 ? 3 : 4;

        // Pick `count` unique random indices
        const indices = [0, 1, 2, 3].sort(() => Math.random() - 0.5).slice(0, count);

        setCells((prev) => {
          const next = [...prev];
          for (const idx of indices) {
            const others = LETTERS.filter((_, i) => i !== idx);
            next[idx] = others[Math.floor(Math.random() * others.length)];
          }
          return next;
        });

        const restore = setTimeout(() => {
          setCells((prev) => {
            const next = [...prev];
            for (const idx of indices) {
              next[idx] = LETTERS[idx];
            }
            return next;
          });
        }, 60 + Math.random() * 80);
        timers.current.push(restore);
        scheduleFlicker();
      }, delay);
      timers.current.push(timer);
    }

    scheduleFlicker();
    return () => timers.current.forEach(clearTimeout);
  }, []);

  // Positional shifts
  useEffect(() => {
    const shiftTimers: ReturnType<typeof setTimeout>[] = [];

    function scheduleShift() {
      const delay = 3000 + Math.random() * 8000;
      const t = setTimeout(() => {
        const group = GROUPS[Math.floor(Math.random() * GROUPS.length)];
        const shift = randomShift();

        // Apply shift
        setOffsets((prev) => {
          const next = [...prev];
          for (const idx of group) {
            next[idx] = { x: shift.x, y: shift.y };
          }
          return next;
        });

        // Snap back
        const restore = setTimeout(() => {
          setOffsets((prev) => {
            const next = [...prev];
            for (const idx of group) {
              next[idx] = { x: 0, y: 0 };
            }
            return next;
          });
        }, 30 + Math.random() * 20);
        shiftTimers.push(restore);

        scheduleShift();
      }, delay);
      shiftTimers.push(t);
    }

    scheduleShift();
    return () => shiftTimers.forEach(clearTimeout);
  }, []);

  return (
    <div className="borg-logo-text grid h-full w-full grid-cols-2 grid-rows-2">
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
