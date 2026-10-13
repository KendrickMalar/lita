import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { forgetAll } from "./hooks/useQuietLoad";
import { SIGN_IN_AGAIN } from "./components/ErrorNote";
import { host, type EngineKind, type EngineStatus, type EnginesStatus, type SessionStatus, type UiError } from "./platform/host";
import { mockScene } from "./platform/mock";
import type { PublishingProfile } from "./platform/types";
import { asUiError } from "./errors";
import { t } from "./i18n";
import { CLAUDE_INSTALL_URL, ENGINE_NAMES, EngineContext } from "./engine";
import { ErrorNote } from "./components/ErrorNote";
import { TopBar } from "./components/TopBar";
import { Shell } from "./components/Shell";
import { Launch } from "./components/Launch";
import { UpdateDialog } from "./components/UpdateDialog";
import { useUpdates } from "./hooks/useUpdates";
import "./App.css";

const CODEX_INSTALL_URL = "https://developers.openai.com/codex/cli";

type Auth = { kind: "checking" } | { kind: "waiting" } | { kind: "done"; status: SessionStatus; error?: UiError };

type ProfileState = { kind: "loading" } | { kind: "error"; error: UiError } | { kind: "ready"; profiles: PublishingProfile[]; selected: PublishingProfile };
export default function App() {
  // The last known state of both engines, and which one is in use.
  const [engines, setEngines] = useState<EnginesStatus | null>(null);
  const [engineChecking, setEngineChecking] = useState(true);
  // Whether the body (sidebar and pages) is open. It opens when the engine in
  // use can write, and stays open if one that cannot is picked from inside it;
  // the menu and the warning then say what is wrong.
  const [bodyOpen, setBodyOpen] = useState(false);
  // The engine-menu mock scenes start inside the body, even when the engine in
  // use cannot write, to show the menu's way to the steps.
  const sceneOpensBody = useRef(mockScene()?.startsWith("engine-menu") ?? false);
  const [auth, setAuth] = useState<Auth>({ kind: "checking" });
  const [profileState, setProfileState] = useState<ProfileState>({ kind: "loading" });
  const profileLoadVersion = useRef(0);
  const loadProfiles = useCallback(async () => {
    const request = ++profileLoadVersion.current;
    setProfileState({ kind: "loading" });
    try {
      const [profiles, selected] = await Promise.all([host.listProfiles(), host.selectedProfile()]);
      if (request === profileLoadVersion.current) setProfileState({ kind: "ready", profiles, selected });
    } catch (e) {
      if (request === profileLoadVersion.current) setProfileState({ kind: "error", error: asUiError(e) });
    }
  }, []);
  // Once both checks have answered, the app is up: a later re-check of the
  // engine shows its own steps, not the launch screen again.
  const [booted, setBooted] = useState(false);
  const stepsRef = useRef<HTMLElement>(null);
  const updates = useUpdates();
  const updateDialog = updates.update && (
    <UpdateDialog state={updates.update} onState={updates.setUpdate} onClose={() => updates.setUpdate(null)} />
  );

  // The native side refreshes the stored session on launch; keep asking
  // until it has decided, so a slow network never shows as "signed out".
  const loadSession = useCallback(async () => {
    for (let i = 0; i < 100; i++) {
      try {
        const status = await host.sessionStatus();
        if (status.status !== "restoring") return setAuth({ kind: "done", status });
      } catch (e) {
        return setAuth({ kind: "done", status: { status: "signed_out" }, error: asUiError(e) });
      }
      await new Promise((r) => setTimeout(r, 250));
    }
    setAuth({ kind: "done", status: { status: "signed_out" } });
  }, []);

  const signIn = useCallback(async () => {
    setAuth({ kind: "waiting" });
    try { setAuth({ kind: "done", status: await host.signIn() }); }
    catch (e) {
      const error = asUiError(e);
      setAuth({ kind: "done", status: { status: "signed_out" }, error: error.code === "cancelled" ? undefined : error });
    }
  }, []);

  const signOut = useCallback(async () => {
    forgetAll();
    profileLoadVersion.current += 1;
    setProfileState({ kind: "loading" });
    try { setAuth({ kind: "done", status: await host.signOut() }); }
    catch (e) { setAuth({ kind: "done", status: { status: "signed_out" }, error: asUiError(e) }); }
  }, []);

  const check = useCallback(async () => {
    setEngineChecking(true);
    try {
      const next = await host.engineStatus();
      setEngines(next);
      setBodyOpen(next[next.selected].status === "ready" || sceneOpensBody.current);
    } catch (e) {
      // Unreadable: the engine in use (the last known one, else Codex) is
      // "could not be checked"; the other keeps what was known of it.
      const failed: EngineStatus = { status: "error", message: String(e) };
      setEngines((prev) => {
        const selected = prev?.selected ?? "codex";
        const next: EnginesStatus = { selected, codex: prev?.codex ?? failed, claude: prev?.claude ?? failed };
        next[selected] = failed;
        return next;
      });
      setBodyOpen(false);
    } finally {
      sceneOpensBody.current = false;
      setEngineChecking(false);
    }
  }, []);

  // Picking an engine saves the choice and moves the mark; the states are not
  // read again. Resolves to the error when the choice could not be saved, and
  // the choice stays as it was.
  const selectEngine = useCallback(async (engine: EngineKind): Promise<UiError | null> => {
    try { await host.selectEngine(engine); }
    catch (e) { return asUiError(e); }
    setEngines((prev) => (prev ? { ...prev, selected: engine } : prev));
    if (engines?.[engine].status === "ready") setBodyOpen(true);
    return null;
  }, [engines]);

  // "サインインし直す" from any error note (#105).
  useEffect(() => {
    const on = () => void signIn();
    window.addEventListener(SIGN_IN_AGAIN, on);
    return () => window.removeEventListener(SIGN_IN_AGAIN, on);
  }, [signIn]);

  useEffect(() => {
    void check();
    void loadSession();
  }, [check, loadSession]);

  const session = auth.kind === "done" ? auth.status : null;
  const signedIn = session?.status === "signed_in";
  const selected: EngineKind = engines?.selected ?? "codex";
  const selectedStatus = engines ? engines[selected] : null;
  useEffect(() => {
    if (!signedIn) {
      profileLoadVersion.current += 1;
      setProfileState({ kind: "loading" });
      return;
    }
    void loadProfiles();
  }, [signedIn, loadProfiles]);
  const onProfilesChanged = useCallback((profiles: PublishingProfile[], selected: PublishingProfile) => {
    setProfileState({ kind: "ready", profiles, selected });
  }, []);

  if (!booted && auth.kind === "done" && !engineChecking) setBooted(true);

  const engineName = ENGINE_NAMES[selected];
  const screen = (): ReactNode => {
    // Nothing has been decided yet, so there is nothing to act on: one picture
    // for the whole launch, with the line swapping as it moves on (#111).
    if (!booted && (auth.kind === "checking" || (signedIn && engineChecking))) {
      return <Launch step={auth.kind === "checking" ? "session" : "engine"} />;
    }

    if (signedIn && bodyOpen) {
      if (profileState.kind !== "ready") {
        return (
          <>
            <TopBar session={session} engines={engines} engineChecking={engineChecking} onSelectEngine={selectEngine} onSignOut={() => void signOut()} onShowEngineSteps={() => void check()} />
            <main className="shell">
              {profileState.kind === "loading" ? (
                <p className="muted" role="status">{t("profile.loading")}</p>
              ) : (
                <section className="panel">
                  <ErrorNote error={profileState.error} />
                  <button className="btn pri" onClick={() => void loadProfiles()}>{t("action.retry")}</button>
                </section>
              )}
            </main>
            {updateDialog}
          </>
        );
      }
      return (
        <>
          <Shell
            session={session}
            engines={engines}
            engineChecking={engineChecking}
            onSelectEngine={selectEngine}
            onSignOut={() => void signOut()}
            onShowEngineSteps={() => void check()}
            updateAvailable={updates.available}
            onUpdate={() => void updates.checkUpdate(true)}
            profiles={profileState.profiles}
            selectedProfile={profileState.selected}
            onProfilesChanged={onProfilesChanged}
          />
          {updateDialog}
        </>
      );
    }

    return (
      <div className="app">
        <TopBar
          session={session}
          engines={engines}
          engineChecking={engineChecking}
          onSelectEngine={selectEngine}
          onSignOut={() => void signOut()}
          onShowEngineSteps={() => stepsRef.current?.scrollIntoView({ behavior: "smooth", block: "start" })}
        />
        <main className="shell">
          {!signedIn && (
            <section className="panel">
              <h2>Lita</h2>
              <p className="sub">{t("app.tagline")}</p>
              <SessionBody auth={auth} onSignIn={() => void signIn()} onCancel={() => void host.cancelSignIn()} />
            </section>
          )}

          {signedIn && (
            <section className="panel" ref={stepsRef}>
              {engineChecking
                ? <p className="muted" role="status">{t("engine.checking", { engine: engineName })}</p>
                : <EngineSteps engine={selected} status={selectedStatus} onRetry={() => void check()} onSelect={selectEngine} />}
            </section>
          )}

          <footer className="privacy">{t("privacy.note", { engine: engineName })}</footer>
        </main>
        {updateDialog}
      </div>
    );
  };

  return <EngineContext.Provider value={selected}>{screen()}</EngineContext.Provider>;
}

function SessionBody({ auth, onSignIn, onCancel }: { auth: Auth; onSignIn: () => void; onCancel: () => void }) {
  if (auth.kind === "checking") return <p className="muted">{t("session.checking")}</p>;
  if (auth.kind === "waiting") {
    return (
      <div className="row between">
        <p className="muted">{t("session.waiting")}</p>
        <button className="btn" onClick={onCancel}>{t("action.cancel")}</button>
      </div>
    );
  }
  return (
    <>
      <p>{t("session.signedOut")}</p>
      {auth.error && <ErrorNote error={auth.error} />}
      <button className="btn pri" onClick={onSignIn}>{t("action.signIn")}</button>
    </>
  );
}

// The way to fix the engine in use, and under it the one line that switches
// to the other. Nothing says which is better, and the other one's state is not
// shown here: the menu has it.
function EngineSteps({ engine, status, onRetry, onSelect }: { engine: EngineKind; status: EngineStatus | null; onRetry: () => void; onSelect: (engine: EngineKind) => Promise<UiError | null> }) {
  if (!status) return null;
  return (
    <>
      {engine === "codex" ? <CodexSteps status={status} onRetry={onRetry} /> : <ClaudeSteps status={status} onRetry={onRetry} />}
      <EngineSwitch to={engine === "codex" ? "claude" : "codex"} onSelect={onSelect} />
    </>
  );
}

function EngineSwitch({ to, onSelect }: { to: EngineKind; onSelect: (engine: EngineKind) => Promise<UiError | null> }) {
  const [error, setError] = useState<UiError | null>(null);
  const busy = useRef(false);
  const choose = async () => {
    if (busy.current) return;
    busy.current = true;
    setError(null);
    try { setError(await onSelect(to)); }
    finally { busy.current = false; }
  };
  return (
    <div className="engine-switch">
      <button className="link quiet-link" onClick={() => void choose()}>{t("engine.use", { engine: ENGINE_NAMES[to] })}</button>
      {error && <ErrorNote error={error} />}
    </div>
  );
}

function CodexSteps({ status, onRetry }: { status: EngineStatus; onRetry: () => void }) {
  switch (status.status) {
    case "ready":
      return null;
    case "not_logged_in":
      return (
        <>
          <h2>{t("codex.notLoggedIn")}</h2>
          <p className="muted">{t("codex.notLoggedIn.hint")}</p>
          <pre>codex login</pre>
          <button className="btn pri" onClick={onRetry}>{t("action.retry")}</button>
        </>
      );
    case "not_installed":
      return (
        <>
          <h2>{t("codex.notInstalled")}</h2>
          <p className="muted">{t("codex.notInstalled.hint")}</p>
          <div className="row">
            <button className="btn pri" onClick={() => void host.openExternal(CODEX_INSTALL_URL)}>{t("codex.install")}</button>
            <button className="btn" onClick={onRetry}>{t("action.retry")}</button>
          </div>
        </>
      );
    case "config_broken":
      return (
        <>
          <h2>{t("codex.configBroken")}</h2>
          <p className="muted">{t("codex.configBroken.hint")}</p>
          <pre>{status.message}</pre>
          <button className="btn pri" onClick={onRetry}>{t("action.retry")}</button>
        </>
      );
    case "error":
      return (
        <>
          <ErrorNote error={{ code: "unknown", detail: status.message }} />
          <button className="btn pri" onClick={onRetry}>{t("action.retry")}</button>
        </>
      );
  }
}

// Claude Code has no configuration state of its own to fix here
// ("config_broken" is Codex only), so that case reads as "could not be checked".
function ClaudeSteps({ status, onRetry }: { status: EngineStatus; onRetry: () => void }) {
  switch (status.status) {
    case "ready":
      return null;
    case "not_logged_in":
      return (
        <>
          <h2>{t("claude.notLoggedIn")}</h2>
          <p className="muted">{t("claude.notLoggedIn.hint")}</p>
          <pre>claude auth login</pre>
          <button className="btn pri" onClick={onRetry}>{t("action.retry")}</button>
        </>
      );
    case "not_installed":
      return (
        <>
          <h2>{t("claude.notInstalled")}</h2>
          <p className="muted">{t("claude.notInstalled.hint")}</p>
          <div className="row">
            <button className="btn pri" onClick={() => void host.openExternal(CLAUDE_INSTALL_URL)}>{t("claude.install")}</button>
            <button className="btn" onClick={onRetry}>{t("action.retry")}</button>
          </div>
        </>
      );
    case "config_broken":
    case "error":
      return (
        <>
          <ErrorNote error={{ code: "unknown", detail: status.message }} />
          <button className="btn pri" onClick={onRetry}>{t("action.retry")}</button>
        </>
      );
  }
}
