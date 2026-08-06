import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { copyText } from "../lib/clipboard";
import { useI18n } from "../i18n/I18nContext";
import { useToast } from "../hooks/useToast";

interface CopyButtonProps {
  value: string;
  label?: string;
}

export function CopyButton({ value, label }: CopyButtonProps) {
  const { t } = useI18n();
  const { toast } = useToast();
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    const ok = await copyText(value);
    if (ok) {
      setCopied(true);
      toast(t("common.copied_to_clipboard"), "success");
      setTimeout(() => setCopied(false), 1600);
    }
  };

  const Icon = copied ? Check : Copy;

  return (
    <button
      type="button"
      onClick={() => void handleCopy()}
      aria-label={label ? undefined : t("common.copy_whisper_id")}
      className="inline-flex items-center gap-1.5 rounded-lg border border-wp-line/10 bg-wp-panel-2 px-3 py-1.5 text-xs font-medium text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
    >
      <Icon className="h-3.5 w-3.5" aria-hidden="true" />
      {label ? <span>{copied ? t("common.copied") : label}</span> : null}
    </button>
  );
}
