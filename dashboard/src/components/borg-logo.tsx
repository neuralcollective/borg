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
  const textSize = size === "desktop" ? "text-[13px]" : "text-[9px]";
  const [cells, setCells] = useState(LETTERS);
  const [offsets, setOffsets] = useState<Offsets>(() =>
    LETTERS.map(() => ({ x: 0, y: 0 }))
  );
  const timers = useRef<ReturnType<typeof setTimeout>[]>([]);

  // Letter flicker
  useEffect(() => {
    function scheduleFlicker(idx: number) {
      const delay = 2000 + Math.random() * 6000;
      const timer = setTimeout(() => {
        const others = LETTERS.filter((_, i) => i !== idx);
        const glitch = others[Math.floor(Math.random() * others.length)];
        setCells((prev) => {
          const next = [...prev];
          next[idx] = glitch;
          return next;
        });
        const restore = setTimeout(() => {
          setCells((prev) => {
            const next = [...prev];
            next[idx] = LETTERS[idx];
            return next;
          });
        }, 60 + Math.random() * 80);
        timers.current.push(restore);
        scheduleFlicker(idx);
      }, delay);
      timers.current.push(timer);
    }

    for (let i = 0; i < LETTERS.length; i++) {
      scheduleFlicker(i);
    }
    return () => timers.current.forEach(clearTimeout);
  }, []);

  // Positional shifts
  useEffect(() => {
    const shiftTimers: ReturnType<typeof setTimeout>[] = [];

    function scheduleShift() {
      const delay = 1500 + Math.random() * 4000;
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
        }, 120 + Math.random() * 200);
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
          className={`flex items-center justify-center ${textSize} text-black`}
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
