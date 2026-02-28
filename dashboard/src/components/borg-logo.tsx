import { useState, useEffect, useRef } from "react";

const WORD = "BORG";
const LETTERS = WORD.split("");

function BorgLogo({ size = "desktop" }: { size?: "desktop" | "mobile" }) {
  const textSize = size === "desktop" ? "text-[13px]" : "text-[9px]";
  const [cells, setCells] = useState(LETTERS);
  const timers = useRef<ReturnType<typeof setTimeout>[]>([]);

  useEffect(() => {
    function scheduleFlicker(idx: number) {
      const delay = 2000 + Math.random() * 6000;
      const timer = setTimeout(() => {
        // Pick a random different letter from BORG
        const others = LETTERS.filter((_, i) => i !== idx);
        const glitch = others[Math.floor(Math.random() * others.length)];
        setCells((prev) => {
          const next = [...prev];
          next[idx] = glitch;
          return next;
        });
        // Snap back after a brief flash
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

  return (
    <div className="borg-logo-text grid h-full w-full grid-cols-2 grid-rows-2">
      {cells.map((c, i) => (
        <span
          key={i}
          className={`flex items-center justify-center ${textSize} text-black`}
        >
          {c}
        </span>
      ))}
    </div>
  );
}

export { BorgLogo };
