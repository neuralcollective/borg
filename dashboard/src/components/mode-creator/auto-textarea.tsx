import { useEffect, useRef } from "react";
import { cn } from "@/lib/utils";

export function AutoTextarea({
  value,
  onChange,
  placeholder,
  className,
  disabled,
  minRows = 3,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  className?: string;
  disabled?: boolean;
  minRows?: number;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    const minH = minRows * 18; // ~18px per line at text-[11px]
    el.style.height = `${Math.max(minH, Math.min(el.scrollHeight, 280))}px`;
  }, [value, minRows]);

  return (
    <textarea
      ref={ref}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      disabled={disabled}
      className={cn(
        "w-full resize-none rounded-md border border-white/[0.08] bg-black/30 px-2.5 py-2 font-mono text-[11px] leading-[1.5] text-zinc-200 outline-none placeholder:text-zinc-700 focus:border-blue-500/40 disabled:opacity-50",
        className
      )}
    />
  );
}
