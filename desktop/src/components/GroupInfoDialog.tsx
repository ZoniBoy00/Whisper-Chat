import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ChangeEvent } from "react";
import {
  ArrowLeftRight,
  Crown,
  ImagePlus,
  Link2,
  Loader2,
  LogOut,
  ShieldCheck,
  Trash2,
  UserMinus,
  UserPlus,
  X,
} from "lucide-react";
import type { ContactInfo, GroupInfo, GroupMember, ProfileInfo } from "../types";
import { cx, isGroupId, mediaUrl, shortPeerId } from "../lib/format";
import { getProfile } from "../lib/relay";
import type { TFunction } from "../i18n/types";
import { useI18n } from "../i18n/I18nContext";
import { useToast } from "../hooks/useToast";
import { Avatar } from "./Avatar";

/** Whisper IDs are 24 lowercase hex characters. */
const PEER_ID_PATTERN = /^[0-9a-f]{24}$/i;
/** Uploaded group photos are capped at 2 MiB, mirroring the relay. */
const MAX_AVATAR_BYTES = 2 * 1024 * 1024;

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
  /** Our own peer ID; used to resolve our role from `owner_peer_id` when the
   *  server-reported `my_role` has not synced yet. */
  myPeerId: string;
  /** Copy the group's shareable join link (any member). */
  onCopyJoinLink: (groupId: string) => Promise<void>;
  onOpenChange: (open: boolean) => void;
  onFetchInfo: (groupId: string) => Promise<GroupInfo>;
  /** Add a member to the group's roster after creation (owner/admin). */
  onAddMember: (groupId: string, peerId: string) => Promise<void>;
  /** Set the group's avatar image (raw base64, ≤2 MB; owner/admin). */
  onSetGroupAvatar: (groupId: string, avatarBase64: string) => Promise<void>;
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
  myPeerId,
  onCopyJoinLink,
  onOpenChange,
  onFetchInfo,
  onAddMember,
  onSetGroupAvatar,
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
  // Add-member input: the Whisper ID of the peer being invited.
  const [addMemberInput, setAddMemberInput] = useState("");
  const [addMemberError, setAddMemberError] = useState<string | null>(null);
  // Group-photo upload errors (type / size validation happens client-side).
  const [avatarError, setAvatarError] = useState<string | null>(null);
  // Profiles fetched for roster members the contact list does not know yet.
  const [memberProfiles, setMemberProfiles] = useState<Record<string, ProfileInfo>>({});

  /** Contact data keyed by peer ID for O(1) roster lookups. */
  const contactsById = useMemo(() => {
    const map = new Map<string, ContactInfo>();
    for (const contact of contacts) map.set(contact.peer_id, contact);
    return map;
  }, [contacts]);

  /** The peers that may be added to the group: ACCEPTED 1:1 contacts only
   *  (pending requests are not chatable and the relay rejects them), excluding
   *  groups and anyone already on the roster. */
  const eligibleContacts = useMemo(() => {
    const memberIds = new Set((info?.members ?? []).map((m) => m.peer_id));
    return contacts.filter(
      (contact) =>
        contact.status !== "pending" &&
        !isGroupId(contact.peer_id) &&
        !memberIds.has(contact.peer_id)
    );
  }, [contacts, info]);

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
      setAddMemberInput("");
      setAddMemberError(null);
      setAvatarError(null);
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
  /** Add a member by Whisper ID (owner/admin): validates the peer ID, runs the
   *  relay call (which shares every member's Megolm key to the newcomer) and
   *  reloads the roster so the new member and count appear immediately. */
  const handleAddMember = async () => {
    if (!groupId) return;
    const peerId = addMemberInput.trim().toLowerCase();
    if (!PEER_ID_PATTERN.test(peerId)) {
      setAddMemberError(t("groupInfo.invalid_peer_id_24"));
      return;
    }
    if (info?.members.some((m) => m.peer_id === peerId)) {
      setAddMemberError(t("groupInfo.member_already_in_group"));
      return;
    }
    setBusyPeer("__add_member__");
    setError(null);
    setAddMemberError(null);
    try {
      await onAddMember(groupId, peerId);
      setAddMemberInput("");
      if (groupId) await reload(groupId);
      toast(t("toast.member_added"), "success");
    } catch (err) {
      const message = String(err).replace(/^Error:\s*/, "");
      setError(message);
      toast(message, "error");
    } finally {
      setBusyPeer(null);
    }
  };

  /** Read a picked photo file, validate type/size and upload it as the group's
   *  avatar (raw base64 without the `data:` prefix, like the profile avatar). */
  const handleAvatarFile = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (!file) return;
    if (!/^image\/(png|jpe?g|webp)$/i.test(file.type)) {
      const message = t("groupInfo.photo_type_error");
      setAvatarError(message);
      toast(message, "error");
      return;
    }
    if (file.size > MAX_AVATAR_BYTES) {
      const message = t("groupInfo.photo_size_error");
      setAvatarError(message);
      toast(message, "error");
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      const result = String(reader.result ?? "");
      const comma = result.indexOf(",");
      void applyAvatar(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.readAsDataURL(file);
    // Allow picking the same file again after a rejected/retried upload.
    event.target.value = "";
  };

  const applyAvatar = async (avatarBase64: string) => {
    if (!groupId) return;
    setBusyPeer("__set_avatar__");
    setError(null);
    setAvatarError(null);
    try {
      await onSetGroupAvatar(groupId, avatarBase64);
      if (groupId) await reload(groupId);
      toast(t("toast.group_avatar_updated"), "success");
    } catch (err) {
      const message = String(err).replace(/^Error:\s*/, "");
      setError(message);
      toast(message, "error");
    } finally {
      setBusyPeer(null);
    }
  };

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

  // Our role gates the admin controls. Prefer the server-reported `my_role`;
  // when it has not synced yet, fall back to `owner_peer_id` (we are the
  // owner) so the owner controls are never hidden behind a stale role.
  const myRole = info?.my_role ?? (info?.owner_peer_id === myPeerId ? "owner" : null);
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
            <Avatar
              name={info?.name ?? undefined}
              size={40}
              src={
                info?.avatar_url ? mediaUrl(relayUrl, info.avatar_url) : null
              }
              variant="group"
            />
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

          {canPromote && info ? (
            <div className="border-t border-wp-line/10 pt-4">
              <p className="mb-2 text-xs leading-snug text-wp-faint">
                {t("groupInfo.add_member_hint")}
              </p>
              {eligibleContacts.length === 0 ? (
                <p className="text-xs leading-snug text-wp-faint">
                  {t("groupInfo.no_contacts_to_add")}
                </p>
              ) : (
                <div className="flex items-center gap-2">
                  <label className="sr-only" htmlFor="group-add-member">
                    {t("groupInfo.add_member")}
                  </label>
                  <select
                    id="group-add-member"
                    value={addMemberInput}
                    onChange={(e) => {
                      setAddMemberInput(e.target.value);
                      setAddMemberError(null);
                    }}
                    disabled={busyPeer !== null}
                    aria-invalid={addMemberError ? true : undefined}
                    aria-describedby={
                      addMemberError ? "group-add-member-error" : undefined
                    }
                    className="min-w-0 flex-1 rounded-xl border border-wp-line/10 bg-wp-panel-3 px-3 py-2 text-xs text-wp-text focus:border-wp-accent focus:outline-none disabled:opacity-40"
                  >
                    <option value="">{t("groupInfo.add_member_placeholder")}</option>
                    {eligibleContacts.map((contact) => (
                      <option key={contact.peer_id} value={contact.peer_id}>
                        {memberName(contact.peer_id)}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    onClick={() => void handleAddMember()}
                    disabled={busyPeer !== null || !addMemberInput.trim()}
                    className="inline-flex shrink-0 items-center gap-1.5 rounded-xl border border-wp-line/10 px-3 py-2 text-xs font-semibold text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text disabled:opacity-40"
                  >
                    {busyPeer === "__add_member__" ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <UserPlus className="h-3.5 w-3.5" aria-hidden="true" />
                    )}
                    {t("groupInfo.add_member")}
                  </button>
                </div>
              )}
              {addMemberError ? (
                <p
                  id="group-add-member-error"
                  role="alert"
                  className="mt-2 text-xs leading-snug text-wp-danger"
                >
                  {addMemberError}
                </p>
              ) : null}
            </div>
          ) : null}

          {canPromote && info ? (
            <div className="border-t border-wp-line/10 pt-4">
              <p className="mb-2 text-xs leading-snug text-wp-faint">
                {t("groupInfo.change_photo_hint")}
              </p>
              <label
                htmlFor="group-avatar-file"
                className="inline-flex cursor-pointer items-center gap-2 rounded-xl border border-wp-line/10 px-3 py-2 text-xs font-semibold text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text"
              >
                <ImagePlus className="h-3.5 w-3.5" aria-hidden="true" />
                {t("groupInfo.change_photo")}
                <input
                  id="group-avatar-file"
                  type="file"
                  accept="image/png,image/jpeg,image/webp"
                  onChange={handleAvatarFile}
                  disabled={busyPeer !== null}
                  className="sr-only"
                />
              </label>
              {busyPeer === "__set_avatar__" ? (
                <Loader2
                  className="ml-2 inline h-3.5 w-3.5 animate-spin text-wp-faint"
                  aria-hidden="true"
                />
              ) : null}
              {avatarError ? (
                <p
                  role="alert"
                  className="mt-2 text-xs leading-snug text-wp-danger"
                >
                  {avatarError}
                </p>
              ) : null}
            </div>
          ) : null}

          <div className="border-t border-wp-line/10 pt-4">
            <p className="mb-2 text-xs leading-snug text-wp-faint">
              {t("groupInfo.join_link_hint")}
            </p>
            <button
              type="button"
              onClick={() => {
                if (!groupId) return;
                void onCopyJoinLink(groupId);
              }}
              disabled={busyPeer !== null || !groupId}
              className="inline-flex items-center gap-2 rounded-xl border border-wp-line/10 px-3 py-2 text-xs font-semibold text-wp-dim transition hover:bg-wp-panel-3 hover:text-wp-text disabled:opacity-40"
            >
              <Link2 className="h-3.5 w-3.5" aria-hidden="true" />
              {t("groupInfo.copy_join_link")}
            </button>
          </div>

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
