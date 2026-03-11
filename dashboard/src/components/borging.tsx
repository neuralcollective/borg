import { useState } from "react";
import "./borging.css";

function pickWorkingLabel() {
  return Math.random() < 0.2 ? "Borging..." : "Working...";
}

export function BorgingIndicator() {
  const [label] = useState(pickWorkingLabel);
  return (
    <div className="flex items-center gap-2 py-1">
      <span className="relative flex h-2 w-2">
        <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-400 opacity-75" />
        <span className="relative inline-flex rounded-full h-2 w-2 bg-amber-400" />
      </span>
      <span className="shimmer-text text-sm font-medium text-amber-400">{label}</span>
    </div>
  );
}

export { pickWorkingLabel };

export function TimelineItem({
  icon,
  label,
  detail,
  isActive,
  isFirst,
  isLast,
}: {
  icon?: React.ReactNode;
  label: string;
  detail?: string;
  isActive?: boolean;
  isFirst?: boolean;
  isLast?: boolean;
}) {
  return (
    <div
      className={`relative flex gap-3 pl-1 py-1.5 animate-[timeline-fade-in_0.3s_ease-out] ${
        isActive ? "bg-amber-500/[0.04] rounded-lg" : ""
      }`}
    >
      {/* Vertical connecting line */}
      <div className="relative flex flex-col items-center w-5 shrink-0">
        {!isFirst && <div className="absolute bottom-1/2 w-px h-full bg-[#2a2520]" />}
        {!isLast && <div className="absolute top-1/2 w-px h-full bg-[#2a2520]" />}
        <div className="relative z-10 flex items-center justify-center w-5 h-5">
          {icon || <div className="w-2 h-2 rounded-full bg-[#6b6459]" />}
        </div>
      </div>

      {/* Content */}
      <div className="min-w-0 flex-1 py-0.5">
        <div className="text-[13px] text-[#e8e0d4] leading-snug truncate">{label}</div>
        {detail && <div className="text-[11px] text-[#6b6459] mt-0.5 truncate">{detail}</div>}
      </div>
    </div>
  );
}
