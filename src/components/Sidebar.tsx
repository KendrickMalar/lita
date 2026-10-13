import { useEffect, useRef, useState } from "react";
import type { ArticleStatus, EngineKind, EnginesStatus, PublishingProfile, SessionStatus, UiError } from "../platform/host";
import { t } from "../i18n";
import type { Route } from "./Shell";
import { ErrorNote } from "./ErrorNote";
import { EngineMenu, needsSteps } from "./EngineMenu";
import { ENGINE_KINDS, enginePill } from "../engine";
import { mockScene } from "../platform/mock";

 type Props = {
  route: Route;
  onRoute: (route: Route) => void;
  onGoTopics: () => void;
  quietNew: boolean;
  session: SessionStatus | null;
  engines: EnginesStatus | null;
  engineChecking: boolean;
  onSelectEngine: (engine: EngineKind) => Promise<UiError | null>;
  onSignOut: () => void;
  onShowEngineSteps: () => void;
  updateAvailable: string | null;
  onUpdate: () => void;
  profiles: PublishingProfile[];
  selectedProfile: PublishingProfile;
  onSelectProfile: (id: string) => Promise<boolean>;
  onCreateProfile: (name: string) => Promise<boolean>;
  onRenameProfile: (id: string, name: string) => Promise<boolean>;
  profileBusy: boolean;
  profileError: UiError | null;
};

export const FILTERS: (ArticleStatus | "all")[] = ["all", "draft"];
type ProfileMenuMode = "closed" | "menu" | "create" | "rename";

export function Sidebar({ route, onRoute, onGoTopics, quietNew, session, engines, engineChecking, onSelectEngine, onSignOut, onShowEngineSteps, updateAvailable, onUpdate, profiles, selectedProfile, onSelectProfile, onCreateProfile, onRenameProfile, profileBusy, profileError }: Props) {
  // Profiles are switched only from the account menu at the bottom (D-80).
  const [profileMode, setProfileMode] = useState<ProfileMenuMode>(() => mockScene() === "profiles-menu" || mockScene()?.startsWith("engine-menu") ? "menu" : mockScene() === "profiles-create" ? "create" : "closed");
  const [profileName, setProfileName] = useState("");
  const profileRef = useRef<HTMLDivElement>(null);
  const profileButtonRef = useRef<HTMLButtonElement>(null);
  const profileInputRef = useRef<HTMLInputElement>(null);
  const profileItems = useRef<Array<HTMLButtonElement | null>>([]);
  // Set after a successful switch; the button can only take focus once it is
  // enabled again, i.e. after profileBusy clears.
  const [refocusButton, setRefocusButton] = useState(false);

  useEffect(() => {
    if (!refocusButton || profileBusy) return;
    setRefocusButton(false);
    profileButtonRef.current?.focus();
  }, [refocusButton, profileBusy]);

  useEffect(() => {
    // Disabled items cannot take focus; wait until the change has finished.
    if (profileBusy) return;
    if (profileMode === "create" || profileMode === "rename") {
      requestAnimationFrame(() => {
        profileInputRef.current?.focus();
        if (profileMode === "rename") profileInputRef.current?.select();
      });
    } else if (profileMode === "menu") {
      requestAnimationFrame(() => {
        const index = profiles.findIndex((profile) => profile.id === selectedProfile.id);
        profileItems.current[index < 0 ? 0 : index]?.focus();
      });
    }
  }, [profileMode, profiles, selectedProfile.id, profileBusy]);

  useEffect(() => {
    const closeOutside = (event: MouseEvent) => {
      if (!profileRef.current?.contains(event.target as Node)) setProfileMode("closed");
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (profileMode === "create" || profileMode === "rename") {
        event.preventDefault();
        setProfileName("");
        setProfileMode("menu");
        return;
      }
      if (profileMode === "menu") {
        event.preventDefault();
        setProfileMode("closed");
        profileButtonRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", closeOutside);
    document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("mousedown", closeOutside); document.removeEventListener("keydown", escape); };
  }, [profileMode]);

  const email = session?.status === "signed_in" ? session.email ?? "" : "";
  // The warning is about the engine in use only; while it is being checked
  // the menu says so and the sidebar stays quiet.
  const selectedEngine = engines?.selected ?? "codex";
  const pill = engineChecking ? null : enginePill(selectedEngine, engines?.[selectedEngine] ?? null, false);
  const inArticles = route.kind === "articles" || route.kind === "article" || route.kind === "topics";
  const filter = route.kind === "articles" ? route.filter : null;
  // Profiles, rename and new, the engines (and the way to fix the one in
  // use, only while it cannot write), then sign out.
  const engineSlotBase = profiles.length + 2;
  const engineSlots = ENGINE_KINDS.length + (needsSteps(engines) ? 1 : 0);
  const signOutSlot = engineSlotBase + engineSlots;
  const itemCount = signOutSlot + 1;
  const itemKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const delta = event.key === "ArrowDown" ? 1 : -1;
      profileItems.current[(index + delta + itemCount) % itemCount]?.focus();
    }
    if (event.key === "Escape") {
      event.preventDefault();
      setProfileMode("closed");
      profileButtonRef.current?.focus();
    }
  };
  const chooseProfile = async (id: string) => {
    if (profileBusy) return;
    if (await onSelectProfile(id)) {
      setProfileMode("closed");
      setRefocusButton(true);
    }
  };
  const submitProfile = async (event: React.FormEvent) => {
    event.preventDefault();
    const name = profileName.trim();
    if (!name || profileBusy) return;
    const saved = profileMode === "create"
      ? await onCreateProfile(name)
      : await onRenameProfile(selectedProfile.id, name);
    if (saved) {
      setProfileName("");
      setProfileMode("menu");
    }
  };
  const openRename = () => {
    setProfileName(selectedProfile.name);
    setProfileMode("rename");
  };
  const closeForm = () => { setProfileName(""); setProfileMode("menu"); };
  const itemRef = (index: number) => (element: HTMLButtonElement | null) => { profileItems.current[index] = element; };

  return (
    <nav className="sidebar" aria-label={t("nav.label")}>
      <div className="side-top">
        <b className="wordmark">Lita</b>
        <button className={quietNew ? "btn sm new" : "btn pri sm new"} onClick={onGoTopics} disabled={profileBusy}>{t("topics.entry")}</button>
      </div>

      <div className="side-group">
        <button className={inArticles ? "side-head on" : "side-head"} disabled={profileBusy} onClick={() => onRoute({ kind: "articles", filter: "all" })}>{t("nav.articles")}</button>
        <ul className="side-list">
          {FILTERS.map((f) => (
            <li key={f}>
              <button className={filter === f ? "side-item on" : "side-item"} aria-current={filter === f ? "page" : undefined} disabled={profileBusy} onClick={() => onRoute({ kind: "articles", filter: f })}>{t(`article.filter.${f}` as const)}</button>
            </li>
          ))}
        </ul>
      </div>
      <div className="side-group">
        <button className={route.kind === "voices" ? "side-head on" : "side-head"} aria-current={route.kind === "voices" ? "page" : undefined} disabled={profileBusy} onClick={() => onRoute({ kind: "voices" })}>{t("nav.voices")}</button>
      </div>

      <div className="side-bottom">
        {updateAvailable && <button className="pill quiet side-pill update-pill" onClick={onUpdate}>{t("update.pill", { v: updateAvailable })}</button>}
        {pill && <div className="pill warn side-pill"><span>{pill}</span><button className="link" onClick={onShowEngineSteps}>{t("action.seeSteps")}</button></div>}
        <div className="acct-wrap" ref={profileRef}>
          <button ref={profileButtonRef} type="button" className="acct" aria-haspopup="menu" aria-expanded={profileMode !== "closed"} aria-label={t("account.menuWithProfile", { name: selectedProfile.name })} disabled={profileBusy} onClick={() => { setProfileMode(profileMode === "closed" ? "menu" : "closed"); }}>
            <span className="avatar" aria-hidden="true">{(email[0] ?? "?").toUpperCase()}</span>
            <span className="acct-text"><span className="acct-profile">{selectedProfile.name}</span><span className="acct-email">{email}</span></span>
          </button>
          {profileMode === "menu" && (
            <div className="profile-menu" role="menu" aria-label={t("account.menu")}>
              {profiles.map((profile, index) => (
                <button key={profile.id} ref={itemRef(index)} type="button" role="menuitemradio" aria-checked={profile.id === selectedProfile.id} className={profile.id === selectedProfile.id ? "profile-menu-item selected" : "profile-menu-item"} disabled={profileBusy} onKeyDown={(event) => itemKeyDown(event, index)} onClick={() => void chooseProfile(profile.id)}>
                  <span className="profile-menu-name">{profile.name}</span>{profile.id === selectedProfile.id && <span className="profile-check" aria-hidden="true">✓</span>}
                </button>
              ))}
              {profileError && <div className="profile-error"><ErrorNote error={profileError} /></div>}
              <div className="profile-separator" role="separator" />
              <button ref={itemRef(profiles.length)} type="button" role="menuitem" className="profile-menu-item profile-action" disabled={profileBusy} onKeyDown={(event) => itemKeyDown(event, profiles.length)} onClick={openRename}>{t("profile.rename")}</button>
              <button ref={itemRef(profiles.length + 1)} type="button" role="menuitem" className="profile-menu-item profile-action" disabled={profileBusy} onKeyDown={(event) => itemKeyDown(event, profiles.length + 1)} onClick={() => { setProfileName(""); setProfileMode("create"); }}>{t("profile.create")}</button>
              <div className="profile-separator" role="separator" />
              <div className="menu-meta">{email}</div>
              <EngineMenu
                engines={engines}
                checking={engineChecking}
                onSelect={onSelectEngine}
                onShowSteps={() => { setProfileMode("closed"); onShowEngineSteps(); }}
                slotBase={engineSlotBase}
                itemRef={itemRef}
                onItemKeyDown={itemKeyDown}
              />
              <div className="profile-separator" role="separator" />
              <button ref={itemRef(signOutSlot)} type="button" role="menuitem" className="profile-menu-item" disabled={profileBusy} onKeyDown={(event) => itemKeyDown(event, signOutSlot)} onClick={() => { setProfileMode("closed"); onSignOut(); }}>{t("action.signOut")}</button>
            </div>
          )}
          {(profileMode === "create" || profileMode === "rename") && (
            <form className="profile-menu profile-form" onSubmit={(event) => void submitProfile(event)} onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); closeForm(); } }}>
              <label className="profile-input-label" htmlFor="profile-name">{t(profileMode === "create" ? "profile.createLabel" : "profile.renameLabel")}</label>
              <input ref={profileInputRef} id="profile-name" value={profileName} onChange={(event) => setProfileName(event.target.value)} placeholder={t("profile.namePlaceholder")} required disabled={profileBusy} />
              <p className="profile-hint">{t(profileMode === "create" ? "profile.createHint" : "profile.renameHint")}</p>
              <div className="profile-form-actions">
                <button type="submit" className="btn pri sm" disabled={profileBusy || !profileName.trim()}>{t(profileMode === "create" ? "profile.create" : "profile.save")}</button>
                <button type="button" className="link" disabled={profileBusy} onClick={closeForm}>{t("action.cancel")}</button>
              </div>
              {profileError && <div className="profile-error"><ErrorNote error={profileError} /></div>}
            </form>
          )}
        </div>
      </div>
    </nav>
  );
}
