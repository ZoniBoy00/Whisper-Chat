import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowLeftRight,
  Crown,
  Loader2,
  LogOut,
  ShieldCheck,
  Trash2,
  UserMinus,
  UserPlus,
  Users,
  X,
} from "lucide-react";
import type { ContactInfo, GroupInfo, GroupMember, ProfileInfo } from "../types";
import { cx, mediaUrl, shortPeerId } from "../lib/format";
import { getProfile } from "../lib/relay";
import type { TFunction } from "../i18n/types";
import { useI18n } from "../i18n/I18nContext";
import { useToast } from "../hooks/useToast";
import { Avatar } from "./Avatar";

/** A member's resolved public identity: the best-known name, username and
 *  avatar path, merged from the roster lookup (contact list + one-time profile
 *  fetch) with a short peer ID as the ultimate fallback. */
interface ResolvedMember {
  display_name: string | null;
  username: string | null;
  avatar_url: string | null;
}

interface GroupInfoDialogProps {
  open: boolean;
  /** The group to show; null clears the panel. */
  groupId: string | null;
  onOpenChange: (open: boolean) => void;
  onFetchInfo: (groupId: string) => Promise<GroupInfo>;
  onPromote: (groupId: string, peerId: string) => Promise<void>;
  onDemote: (groupId: string, peerId: string) => Promise<void>;
  onRemove: (groupId: string, peerId: string) => Promise<void>;
  onLeave: (groupId: string) => Promise<void>;
  onTransferOwnership: (groupId: string, peerId: string) => Promise<void>;
  /** Known contact profiles (display name, username, avatar); used to resolve
   *  roster member names without a per-member round-trip. */
  contacts: ContactInfo[];
  /** Relay endpoint; used to resolve `/media/{hash}` member avatar paths. */
  relayUrl: string;
}

/**
 * Module-level member profile cache so re-opening the panel (or reloading the
 * roster after an admin action) never repeats a `get_profile` round-trip for
 * the same member. `contacts` data — kept fresh by `contact-updated` events —
 * takes precedence; this cache only backs members the contact list does not
 * know yet.
 */
const memberProfileCache = new Map<string, ProfileInfo>();
const memberProfileInflight = new Set<string>();
/** Members a `get_profile` lookup proved to have no public profile; never
 *  re-fetched until the dialog's session ends. */
const memberProfileMissing = new Set<string>();

/** A role badge. The owner badge is yellow/highlighted per WhatsApp/Signal
 *  convention; admins get a subtle shield badge. */
function RoleBadge({ role, t }: { role: GroupMember["role"]; t: TFunction }) {
  if (role === "owner") {
    return (
      <span className="inline-flex items-center gap-1 rounded-full bg-[rgb(var(--wp-owner))] px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide text-[rgb(var(--wp-owner-fg))]">
        <Crown className="h-3 w-3" aria-hidden="true" />
        {t("common.owner")}
      </span>
    );
  }
  if (role === "admin") {
    return (
      <span className="inline-flex items-center gap-1 rounded-full bg-wp-panel-3 px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide text-wp-dim">
        <ShieldCheck className="h-3 w-3" aria-hidden="true" />
        {t("common.admin")}
      </span>
    );
  }
  return (
    <span className="rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-wp-faint">
      {t("common.member")}
    </span>
  );
}

export function GroupInfoDialog({
  open,
  groupId,
  onOpenChange,
  onFetchInfo,
  onPromote,
  onDemote,
  onRemove,
  onLeave,
  onTransferOwnership,
  contacts,
  relayUrl,
}: GroupInfoDialogProps) {
  const { t } = useI18n();
  const { toast } = useToast();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [info, setInfo] = useState<GroupInfo | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyPeer, setBusyPeer] = useState<string | null>(null);
  const [confirmingLeave, setConfirmingLeave] = useState(false);
  const [transferTarget, setTransferTarget] = useState("");
  const [confirmingTransfer, setConfirmingTransfer] = useState(false);
  // Profiles fetched for roster members the contact list does not know yet.
  const [memberProfiles, setMemberProfiles] = useState<Record<string, ProfileInfo>>({});

  /** Contact data keyed by peer ID for O(1) roster lookups. */
  const contactsById = useMemo(() => {
    const map = new Map<string, ContactInfo>();
    for (const contact of contacts) map.set(contact.peer_id, contact);
    return map;
  }, [contacts]);

  /** For roster members whose name the contact list does not know yet, fetch
   *  their public profile once and cache it. The row renders the short peer ID
   *  until the profile lands, then upgrades in place. */
  useEffect(() => {
    const members = info?.members ?? [];
    const pending = new Set<string>();
    for (const member of members) {
      if (memberProfileCache.has(member.peer_id)) continue;
      if (memberProfileMissing.has(member.peer_id)) continue;
      if (contactsById.get(member.peer_id)?.display_name) continue;
      pending.add(member.peer_id);
    }
    if (pending.size === 0) return;
    let cancelled = false;
    for (const id of pending) {
      if (memberProfileInflight.has(id)) continue;
      memberProfileInflight.add(id);
      getProfile(id)
        .then((profile) => {
          memberProfileCache.set(id, profile);
          if (!cancelled) {
            setMemberProfiles((prev) => ({ ...prev, [id]: profile }));
          }
        })
        .catch(() => {
          // `no_profile` (unregistered) or a transient lookup failure — the
          // member keeps the short peer ID, and (when provably unregistered)
          // is not re-fetched on every panel open.
          memberProfileMissing.add(id);
        })
        .finally(() => {
          memberProfileInflight.delete(id);
        });
    }
    return () => {
      cancelled = true;
    };
  }, [info, contactsById]);

  /** Best-known identity of a roster member: profile fetch first, then the
   *  contact list, then null (callers fall back to a short peer ID). */
  const resolveMember = useCallback(
    (peerId: string): ResolvedMember => {
      const profile = memberProfiles[peerId] ?? memberProfileCache.get(peerId);
      const contact = contactsById.get(peerId);
      return {
        display_name: profile?.display_name ?? contact?.display_name ?? null,
        username: profile?.username ?? contact?.username ?? null,
        avatar_url: profile?.avatar_url ?? contact?.avatar_url ?? null,
      };
    },
    [memberProfiles, contactsById]
  );

  /** Display label for a member: display name, else @username, else a short
   *  peer ID — exactly what the transfer select and the roster rows show. */
  const memberName = useCallback(
    (peerId: string): string => {
      const { display_name, username } = resolveMember(peerId);
      if (display_name) return display_name;
      if (username) return `@${username}`;
      return shortPeerId(peerId, 16);
    },
    [resolveMember]
  );

  const reload = useCallback(
    async (id: string) => {
      setLoading(true);
      setError(null);
      try {
        setInfo(await onFetchInfo(id));
      } catch (err) {
        setError(String(err).replace(/^Error:\s*/, ""));
      } finally {
        setLoading(false);
      }
    },
    [onFetchInfo]
  );

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      setConfirmingLeave(false);
      setConfirmingTransfer(false);
      setTransferTarget("");
      setError(null);
      dialog.showModal();
      if (groupId) void reload(groupId);
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open, groupId, reload]);

  const close = () => {
    if (busyPeer) return;
    onOpenChange(false);
  };

  const runAction = async (action: () => Promise<void>, peerId: string) => {
    setBusyPeer(peerId);
    setError(null);
    try {
      await action();
      if (groupId) await reload(groupId);
    } catch (err) {
      const message = String(err).replace(/^Error:\s*/, "");
      setError(message);
      toast(message, "error");
    } finally {
      setBusyPeer(null);
    }
  };

  const handleLeave = async () => {
    if (!groupId) return;
    if (!confirmingLeave) {
      setConfirmingLeave(true);
      return;
    }
    setBusyPeer("__leave__");
    setError(null);
    try {
      await onLeave(groupId);
      onOpenChange(false);
    } catch (err) {
      const message = String(err).replace(/^Error:\s*/, "");
      setError(message);
      toast(message, "error");
      setConfirmingLeave(false);
    } finally {
      setBusyPeer(null);
    }
  };

  /** Transfer ownership to the selected member: a first click arms the
   *  confirmation (like leaving), a second click executes. On success the
   *  dialog reloads so the caller's role badge and the transfer control
   *  reflect the new owner right away. */
  const handleTransfer = async () => {
    if (!groupId || !transferTarget) return;
    if (!confirmingTransfer) {
      setConfirmingTransfer(true);
      return;
    }
    setBusyPeer("__transfer__");
    setError(null);
    try {
      await onTransferOwnership(groupId, transferTarget);
      setTransferTarget("");
      setConfirmingTransfer(false);
      if (groupId) await reload(groupId);
      toast(t("toast.group_transferred"), "success");
    } catch (err) {
      const message = String(err).replace(/^Error:\s*/, "");
      setError(message);
      toast(message, "error");
      setConfirmingTransfer(false);
    } finally {
      setBusyPeer(null);
    }
  };

  if (!groupId) return null;

  const myRole = info?.my_role ?? null;
  const canPromote = myRole === "owner" || myRole === "admin";
  const canManage = myRole === "owner";

  return (
    <dialog
      ref={dialogRef}
      className="wp-dialog"
      aria-labelledby="group-info-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
    >
      <div className="w-[min(94vw,30rem)] rounded-2xl bg-wp-panel-2">
        <div className="flex items-center justify-between gap-4 border-b border-wp-line/10 px-5 py-4">
          <div className="flex items-center gap-3">
            <div className="rounded-xl bg-wp-panel-3 p-2 text-wp-accent">
              <Users className="h-4 w-4" aria-hidden="true" />
            </div>
            <div>
              <h2
                id="group-info-title"
                className="font-display text-lg font-semibold tracking-tight text-wp-text"
              >
                {info?.name ?? t("common.group_info")}
              </h2>
              {info ? (
                <p className="text-xs text-wp-faint">
                  {t("common.members_count", { n: info.members.length })}
                </p>
              ) : null}
            </div>
          </div>
          <button
            type="button"
            onClick={close}
            aria-label={t("groupInfo.close_group_info")}
            className="rounded-lg p-2 text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        </div>

        <div className="max-h-[70vh] space-y-4 overflow-y-auto px-5 py-5">
          {loading && !info ? (
            <div className="flex items-center justify-center gap-2 py-10 text-wp-faint">
              <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
              {t("groupInfo.loading_members")}
            </div>
          ) : info ? (
            <ul className="flex flex-col gap-1" aria-label={t("groupInfo.group_members")}>
              {info.members.map((member) => {
                const busy = busyPeer === member.peer_id;
                const resolved = resolveMember(member.peer_id);
                const name = memberName(member.peer_id);
                const avatarSrc = resolved.avatar_url
                  ? mediaUrl(relayUrl, resolved.avatar_url)
                  : null;
                return (
                  <li
                    key={member.peer_id}
                    className="flex items-center gap-3 rounded-xl px-2 py-2.5 transition hover:bg-wp-panel-3"
                  >
                    <Avatar name={name} size={36} src={avatarSrc} />
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-medium text-wp-text">
                        {name}
                      </p>
                      {resolved.username ? (
                        <p className="truncate font-mono text-xs text-wp-faint">
                          @{resolved.username}
                        </p>
                      ) : (
                        <p className="truncate font-mono text-[10px] text-wp-faint">
                          {shortPeerId(member.peer_id, 24)}
                        </p>
                      )}
                    </div>
                    <RoleBadge role={member.role} t={t} />
                    {canPromote && member.role === "member" ? (
                      <button
                        type="button"
                        onClick={() =>
                          void runAction(
                            () => onPromote(groupId, member.peer_id),
                            member.peer_id
                          )
                        }
                        disabled={busy}
                        title={t("groupInfo.make_admin")}
                        aria-label={t("groupInfo.make_admin_aria", { peerId: member.peer_id })}
                        className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-accent disabled:opacity-40"
                      >
                        {busy ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <UserPlus className="h-4 w-4" />
                        )}
                      </button>
                    ) : null}
                    {canManage && member.role === "admin" ? (
                      <button
                        type="button"
                        onClick={() =>
                          void runAction(
                            () => onDemote(groupId, member.peer_id),
                            member.peer_id
                          )
                        }
                        disabled={busy}
                        title={t("groupInfo.demote_from_admin")}
                        aria-label={t("groupInfo.demote_aria", { peerId: member.peer_id })}
                        className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-danger disabled:opacity-40"
                      >
                        {busy ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <UserMinus className="h-4 w-4" />
                        )}
                      </button>
                    ) : null}
                    {canManage && member.role !== "owner" ? (
                      <button
                        type="button"
                        onClick={() =>
                          void runAction(
                            () => onRemove(groupId, member.peer_id),
                            member.peer_id
                          )
                        }
                        disabled={busy}
                        title={t("groupInfo.remove_from_group")}
                        aria-label={t("groupInfo.remove_from_group_aria", { peerId: member.peer_id })}
                        className="rounded-lg p-1.5 text-wp-dim transition hover:bg-wp-panel-2 hover:text-wp-danger disabled:opacity-40"
                      >
                        {busy ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <Trash2 className="h-4 w-4" />
                        )}
                      </button>
                    ) : null}
                  </li>
                );
              })}
            </ul>
          ) : null}

          {error ? (
            <p role="alert" className="text-xs leading-snug text-wp-danger">
              {error}
            </p>
          ) : null}

          <div className="border-t border-wp-line/10 pt-4">
            {canManage && info && info.members.some((m) => m.role !== "owner") ? (
              <div className="mb-4">
                <p className="mb-2 text-xs leading-snug text-wp-faint">
                  {t("groupInfo.transfer_ownership_hint")}
                </p>
                <div className="flex items-center gap-2">
                  <label
                    className="sr-only"
                    htmlFor="transfer-owner-select"
                  >
                    {t("groupInfo.transfer_owner_select_aria")}
                  </label>
                  <select
                    id="transfer-owner-select"
                    value={transferTarget}
                    onChange={(e) => {
                      setTransferTarget(e.target.value);
                      setConfirmingTransfer(false);
                    }}
                    disabled={busyPeer !== null}
                    className="min-w-0 flex-1 rounded-xl border border-wp-line/10 bg-wp-panel-3 px-3 py-2 text-xs text-wp-text focus:border-wp-accent focus:outline-none disabled:opacity-40"
                  >
                    <option value="">{t("groupInfo.transfer_owner_placeholder")}</option>
                    {info.members
                      .filter((member) => member.role !== "owner")
                      .map((member) => (
                        <option key={member.peer_id} value={member.peer_id}>
                          {memberName(member.peer_id)}
                        </option>
                      ))}
                  </select>
                  <button
                    type="button"
                    onClick={() => void handleTransfer()}
                    disabled={busyPeer !== null || !transferTarget}
                    className={cx(
                      "inline-flex shrink-0 items-center justify-center gap-2 rounded-xl px-3 py-2 text-xs font-semibold transition",
                      confirmingTransfer
                        ? "bg-wp-danger/15 text-wp-danger"
                        : "border border-wp-line/10 text-wp-dim hover:bg-wp-panel-3 hover:text-wp-text disabled:opacity-40"
                    )}
                  >
                    {busyPeer === "__transfer__" ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <ArrowLeftRight className="h-3.5 w-3.5" aria-hidden="true" />
                    )}
                    {confirmingTransfer
                      ? t("common.confirm_again")
                      : t("groupInfo.transfer_ownership")}
                  </button>
                </div>
              </div>
            ) : null}
            {myRole === "owner" ? (
              <p className="mb-2 text-xs leading-snug text-wp-faint">
                {t("groupInfo.leave_group_owner_hint")}
              </p>
            ) : null}
            <button
              type="button"
              onClick={() => void handleLeave()}
              disabled={busyPeer !== null}
              className={cx(
                "inline-flex w-full items-center justify-center gap-2 rounded-xl px-4 py-2.5 text-sm font-semibold transition",
                confirmingLeave
                  ? "bg-wp-danger/15 text-wp-danger"
                  : "border border-wp-line/10 text-wp-danger hover:bg-wp-danger/10"
              )}
            >
              {busyPeer === "__leave__" ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <LogOut className="h-4 w-4" aria-hidden="true" />
              )}
              {confirmingLeave ? t("common.confirm_again") : t("groupInfo.leave_group")}
            </button>
          </div>
        </div>
      </div>
    </dialog>
  );
}
