import { useState } from "react";
import { Check, Fingerprint, Link2 } from "lucide-react";
import { CopyButton } from "./CopyButton";
import { copyText } from "../lib/clipboard";
import { getInviteLink } from "../lib/relay";
import { useI18n } from "../i18n/I18nContext";

interface PeerIdCardProps {
  peerId: string;
}

export function PeerIdCard({ peerId }: PeerIdCardProps) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);

  const handleShare = async () => {
    try {
      const link = await getInviteLink();
      const ok = await copyText(link);
      if (ok) {
        setCopied(true);
        setTimeout(() => setCopied(false), 1600);
      }
    } catch {
      // Best-effort: the invite link needs a loaded identity; the peer ID
      // copy button below always works.
    }
  };

  return (
    <div className="w-full rounded-2xl border border-white/10 bg-white/5 p-6 backdrop-blur">
      <div className="flex items-center justify-center gap-2 text-slate-400">
        <Fingerprint className="h-4 w-4" />
        <p className="text-xs font-medium uppercase tracking-widest">{t("common.your_whisper_id")}</p>
      </div>
      <p className="mt-4 select-all break-all font-mono text-2xl font-semibold tracking-wider text-slate-100">
        {peerId}
      </p>
      <div className="mt-5 flex justify-center gap-2">
        <CopyButton value={peerId} label={t("common.copy")} />
        <button
          type="button"
          onClick={() => void handleShare()}
          className="inline-flex items-center gap-1.5 rounded-lg border border-white/15 bg-white/10 px-3 py-1.5 text-xs font-medium text-slate-100 transition hover:bg-white/20 active:scale-95"
        >
          {copied ? (
            <Check className="h-3.5 w-3.5" aria-hidden="true" />
          ) : (
            <Link2 className="h-3.5 w-3.5" aria-hidden="true" />
          )}
          {copied ? t("common.copied") : t("common.share_invite")}
        </button>
      </div>
    </div>
  );
}
