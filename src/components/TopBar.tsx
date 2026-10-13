import { useEffect, useRef, useState } from "react";
import type { EngineKind, EnginesStatus, SessionStatus, UiError } from "../platform/host";
import { t } from "../i18n";
import { enginePill } from "../engine";
import { EngineMenu } from "./EngineMenu";

type Props = {
  session: SessionStatus | null;
  engines: EnginesStatus | null;
  engineChecking: boolean;
  onSelectEngine: (engine: EngineKind) => Promise<UiError | null>;
  onSignOut: () => void;
  onShowEngineSteps: () => void;
};

// Quiet when everything is fine; a labelled pill only when something is not.
export function TopBar({ session, engines, engineChecking, onSelectEngine, onSignOut, onShowEngineSteps }: Props) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) setOpen(false); };
    const esc = (e: KeyboardEvent) => { if (e.key === "Escape") setOpen(false); };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => { document.removeEventListener("mousedown", close); document.removeEventListener("keydown", esc); };
  }, [open]);

  const signedIn = session?.status === "signed_in";
  const email = signedIn && session.status === "signed_in" ? session.email ?? "" : "";
  const initial = (email[0] ?? "?").toUpperCase();
  const selectedEngine = engines?.selected ?? "codex";
  const pill = enginePill(selectedEngine, engines?.[selectedEngine] ?? null, engineChecking);

  return (
    <header className="topbar">
      <b className="wordmark">Lita</b>
      <div className="topbar-right">
        {signedIn && pill && (
          <span className={engineChecking ? "pill quiet" : "pill warn"}>
            {pill}
            {!engineChecking && <button className="link" onClick={onShowEngineSteps}>{t("action.seeSteps")}</button>}
          </span>
        )}
        {signedIn && (
          <div className="acct-wrap" ref={ref}>
            <button className="acct" aria-haspopup="menu" aria-expanded={open} aria-label={t("account.menu")} onClick={() => setOpen((o) => !o)}>
              <span className="avatar" aria-hidden="true">{initial}</span>
              <span className="caret" aria-hidden="true">▾</span>
            </button>
            {open && (
              <div className="menu" role="menu">
                <div className="menu-meta">{email}</div>
                <EngineMenu
                  engines={engines}
                  checking={engineChecking}
                  onSelect={onSelectEngine}
                  onShowSteps={() => { setOpen(false); onShowEngineSteps(); }}
                />
                <div className="profile-separator" role="separator" />
                <button role="menuitem" className="menu-item" onClick={() => { setOpen(false); onSignOut(); }}>{t("action.signOut")}</button>
              </div>
            )}
          </div>
        )}
      </div>
    </header>
  );
}
