import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { copyText } from "../lib/clipboard";

interface CopyButtonProps {
  value: string;
  label?: string;
}

export function CopyButton({ value, label }: CopyButtonProps) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    const ok = await copyText(value);
    if (ok) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    }
  };

  const Icon = copied ? Check : Copy;

  return (
    <button
      type="button"
      onClick={() => void handleCopy()}
      className="inline-flex items-center gap-1.5 rounded-lg border border-white/10 bg-white/5 px-3 py-1.5 text-xs font-medium text-slate-300 transition hover:bg-white/10 hover:text-white"
    >
      <Icon className="h-3.5 w-3.5" />
      {label ? <span>{copied ? "Copied" : label}</span> : null}
    </button>
  );
}
