import { Fingerprint } from "lucide-react";
import { CopyButton } from "./CopyButton";
import { useI18n } from "../i18n/I18nContext";

interface PeerIdCardProps {
  peerId: string;
}

export function PeerIdCard({ peerId }: PeerIdCardProps) {
  const { t } = useI18n();
  return (
    <div className="w-full rounded-2xl border border-white/10 bg-white/5 p-6 backdrop-blur">
      <div className="flex items-center justify-center gap-2 text-slate-400">
        <Fingerprint className="h-4 w-4" />
        <p className="text-xs font-medium uppercase tracking-widest">{t("common.your_whisper_id")}</p>
      </div>
      <p className="mt-4 select-all break-all font-mono text-2xl font-semibold tracking-wider text-slate-100">
        {peerId}
      </p>
      <div className="mt-5 flex justify-center">
        <CopyButton value={peerId} label={t("common.copy")} />
      </div>
    </div>
  );
}
