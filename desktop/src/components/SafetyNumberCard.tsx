import { useEffect, useState } from "react";
import QRCode from "qrcode";
import { BadgeCheck, ShieldCheck, ShieldX } from "lucide-react";
import type { SafetyNumberInfo } from "../types";
import { getSafetyNumber, markContactVerified } from "../lib/relay";
import { cx } from "../lib/format";
import { useI18n } from "../i18n/I18nContext";

interface SafetyNumberCardProps {
  peerId: string;
  /** Display name for the "compare with {name}" hint. */
  displayName: string;
}

/** The verification QR payload: peer ID + the short safety tag. A second
 *  device (or a future mobile app) can compare/scan this out of band. */
function verificationPayload(peerId: string, short: string): string {
  return `whisper://verify?peer=${peerId}&safety=${short}`;
}

/** Signal-style safety number card: QR + grouped digits + verify toggle. */
export function SafetyNumberCard({ peerId, displayName }: SafetyNumberCardProps) {
  const { t } = useI18n();
  const [info, setInfo] = useState<SafetyNumberInfo | null>(null);
  const [qrUrl, setQrUrl] = useState<string | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  const [updating, setUpdating] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setInfo(null);
    setQrUrl(null);
    setUnavailable(false);
    getSafetyNumber(peerId)
      .then(async (result) => {
        if (cancelled) return;
        setInfo(result);
        const url = await QRCode.toDataURL(
          verificationPayload(peerId, result.short),
          {
            width: 168,
            margin: 1,
            color: { dark: "#0b1220", light: "#ffffff" },
          }
        );
        if (!cancelled) setQrUrl(url);
      })
      .catch(() => {
        // `PeerKeyUnknown` until a chat has been started with the peer.
        if (!cancelled) setUnavailable(true);
      });
    return () => {
      cancelled = true;
    };
  }, [peerId]);

  const toggleVerified = async () => {
    if (!info || updating) return;
    const next = !info.verified;
    setUpdating(true);
    try {
      await markContactVerified(peerId, next);
      setInfo({ ...info, verified: next });
    } catch {
      // Best-effort: the flag stays as it was on failure.
    } finally {
      setUpdating(false);
    }
  };

  if (unavailable) {
    return (
      <div className="rounded-xl border border-wp-line/10 bg-wp-panel-3 px-4 py-4">
        <p className="flex items-center gap-2 text-xs font-semibold text-wp-dim">
          <ShieldCheck className="h-4 w-4 text-wp-accent" aria-hidden="true" />
          {t("safety.title")}
        </p>
        <p className="mt-2 text-xs leading-relaxed text-wp-faint">
          {t("safety.unknown_key")}
        </p>
      </div>
    );
  }

  if (!info) {
    return (
      <div className="rounded-xl border border-wp-line/10 bg-wp-panel-3 px-4 py-5">
        <p className="text-center text-xs text-wp-faint">{t("safety.loading")}</p>
      </div>
    );
  }

  return (
    <div className="rounded-xl border border-wp-line/10 bg-wp-panel-3 px-4 py-4">
      <div className="flex items-center justify-between gap-2">
        <p className="flex items-center gap-2 text-xs font-semibold text-wp-dim">
          <ShieldCheck className="h-4 w-4 text-wp-accent" aria-hidden="true" />
          {t("safety.title")}
        </p>
        {info.verified ? (
          <span className="inline-flex items-center gap-1 rounded-full bg-wp-online/15 px-2.5 py-0.5 text-xs font-semibold text-wp-online">
            <BadgeCheck className="h-3.5 w-3.5" aria-hidden="true" />
            {t("safety.verified")}
          </span>
        ) : null}
      </div>

      <div className="mt-3 flex items-center gap-4">
        {qrUrl ? (
          <img
            src={qrUrl}
            alt={t("safety.qr_alt")}
            className="h-28 w-28 shrink-0 rounded-lg bg-white p-1.5"
            width={112}
            height={112}
          />
        ) : (
          <div className="h-28 w-28 shrink-0 rounded-lg bg-white" aria-hidden="true" />
        )}
        <div className="min-w-0 flex-1">
          <p className="select-all font-mono text-[0.95rem] font-semibold leading-snug tracking-tight text-wp-text">
            {info.safety_number}
          </p>
          <p className="mt-1 font-mono text-xs text-wp-faint">
            {t("safety.short", { tag: info.short })}
          </p>
        </div>
      </div>

      <p className="mt-3 text-xs leading-relaxed text-wp-faint">
        {t("safety.verify_hint", { name: displayName })}
      </p>

      <button
        type="button"
        onClick={() => void toggleVerified()}
        disabled={updating}
        className={cx(
          "mt-3 inline-flex w-full items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-sm font-semibold transition active:scale-[0.98] disabled:opacity-50",
          info.verified
            ? "border border-wp-line/10 text-wp-dim hover:bg-wp-panel-2"
            : "bg-wp-accent/15 text-wp-accent hover:bg-wp-accent/25"
        )}
      >
        {info.verified ? (
          <ShieldX className="h-4 w-4" aria-hidden="true" />
        ) : (
          <ShieldCheck className="h-4 w-4" aria-hidden="true" />
        )}
        {info.verified ? t("safety.unverify") : t("safety.verify")}
      </button>
    </div>
  );
}
