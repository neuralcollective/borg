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
    const minH = minRows * 20;
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
        "w-full resize-none rounded-lg border border-[#2a2520] bg-[#0f0e0c] px-3 py-2.5 font-mono text-[12px] leading-[1.6] text-[#e8e0d4] outline-none placeholder:text-[#6b6459] focus:border-amber-500/30 disabled:opacity-50 disabled:cursor-not-allowed transition-colors",
        className
      )}
    />
  );
}
