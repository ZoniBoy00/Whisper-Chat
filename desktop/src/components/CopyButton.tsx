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
      className="inline-flex items-center gap-1.5 rounded-lg border border-wp-line/10 bg-wp-panel-2 px-3 py-1.5 text-xs font-medium text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
    >
      <Icon className="h-3.5 w-3.5" />
      {label ? <span>{copied ? "Copied" : label}</span> : null}
    </button>
  );
}
