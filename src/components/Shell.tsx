import { useCallback, useEffect, useRef, useState } from "react";
import { host, type ArticleStatus, type EngineKind, type EnginesStatus, type PublishingProfile, type SessionStatus, type UiError } from "../platform/host";
import { mockScene } from "../platform/mock";
import { asUiError } from "../errors";
import { Sidebar, FILTERS } from "./Sidebar";
import { ArticleList } from "./ArticleList";
import { TopicPicker } from "./TopicPicker";
import { Editor } from "./Editor";
import { VoiceSection } from "./VoiceSection";

export type Route =
  | { kind: "articles"; filter: ArticleStatus | "all" }
  | { kind: "article"; id: string }
  | { kind: "topics" }
  | { kind: "voices"; voiceId?: string; backTo?: { kind: "article"; id: string } };

const ROUTE_KEY = "lita.route";
function initialRoute(profileId: string): Route {
  const scene = mockScene();
  if (scene?.startsWith("topics")) return { kind: "topics" };
  if (scene?.startsWith("editor")) return { kind: "article", id: "a1" };
  if (scene?.startsWith("voice") || scene?.startsWith("intake") || scene === "building") return { kind: "voices" };
  if (scene?.startsWith("articles")) return { kind: "articles", filter: "all" };
  try {
    const raw = sessionStorage.getItem(`${ROUTE_KEY}:${profileId}`);
    if (raw) {
      const route = JSON.parse(raw) as Route;
      if (route.kind === "articles" && !FILTERS.includes(route.filter)) return { kind: "articles", filter: "all" };
      return route;
    }
  } catch { /* A missing or old route starts at the article list. */ }
  return { kind: "articles", filter: "all" };
}

type Props = {
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
  onProfilesChanged: (profiles: PublishingProfile[], selected: PublishingProfile) => void;
};

export function Shell({ session, engines, engineChecking, onSelectEngine, onSignOut, onShowEngineSteps, updateAvailable, onUpdate, profiles, selectedProfile, onProfilesChanged }: Props) {
  const [route, setRoute] = useState<Route>(() => initialRoute(selectedProfile.id));
  const [mainPrimary, setMainPrimary] = useState(false);
  const [profileChanging, setProfileChanging] = useState(false);
  const [profileError, setProfileError] = useState<UiError | null>(null);
  const changingRef = useRef(false);
  const editorFlush = useRef<(() => Promise<boolean>) | null>(null);
  const registerEditorFlush = useCallback((flush: (() => Promise<boolean>) | null) => { editorFlush.current = flush; }, []);
  useEffect(() => { try { sessionStorage.setItem(`${ROUTE_KEY}:${selectedProfile.id}`, JSON.stringify(route)); } catch { /* Route memory is best-effort. */ } }, [route, selectedProfile.id]);

  const started = useRef(false);
  useEffect(() => {
    if (started.current || session?.status !== "signed_in") return;
    started.current = true;
    void host.startQueue().catch(() => {});
  }, [session]);

  const changeProfile = useCallback(async (operation: () => Promise<PublishingProfile>): Promise<boolean> => {
    if (changingRef.current) return false;
    changingRef.current = true;
    setProfileChanging(true);
    setProfileError(null);
    try {
      const flushed = await editorFlush.current?.() ?? true;
      if (!flushed) return false;
      const selected = await operation();
      const nextProfiles = profiles.filter((profile) => profile.id !== selected.id).concat(selected);
      onProfilesChanged(nextProfiles, selected);
      setRoute({ kind: "articles", filter: "all" });
      return true;
    } catch (error) {
      setProfileError(asUiError(error));
      return false;
    } finally {
      changingRef.current = false;
      setProfileChanging(false);
    }
  }, [profiles, onProfilesChanged]);

  const selectProfile = useCallback((id: string) => {
    if (id === selectedProfile.id) return Promise.resolve(true);
    return changeProfile(() => host.selectProfile(id));
  }, [changeProfile, selectedProfile.id]);

  const createProfile = useCallback((name: string) => changeProfile(() => host.createProfile(name)), [changeProfile]);

  const renameProfile = useCallback(async (id: string, name: string): Promise<boolean> => {
    if (changingRef.current) return false;
    setProfileError(null);
    try {
      await host.renameProfile(id, name);
      const nextProfiles = profiles.map((profile) => profile.id === id ? { ...profile, name } : profile);
      const nextSelected = selectedProfile.id === id ? { ...selectedProfile, name } : selectedProfile;
      onProfilesChanged(nextProfiles, nextSelected);
      return true;
    } catch (error) {
      setProfileError(asUiError(error));
      return false;
    }
  }, [profiles, selectedProfile, onProfilesChanged]);

  return (
    <div className="frame">
      <Sidebar
        route={route}
        onRoute={setRoute}
        quietNew={mainPrimary || route.kind === "topics"}
        onGoTopics={() => setRoute({ kind: "topics" })}
        session={session}
        engines={engines}
        engineChecking={engineChecking}
        onSelectEngine={onSelectEngine}
        onSignOut={onSignOut}
        onShowEngineSteps={onShowEngineSteps}
        updateAvailable={updateAvailable}
        onUpdate={onUpdate}
        profiles={profiles}
        selectedProfile={selectedProfile}
        onSelectProfile={selectProfile}
        onCreateProfile={createProfile}
        onRenameProfile={renameProfile}
        profileBusy={profileChanging}
        profileError={profileError}
      />
      <main className="main" key={selectedProfile.id}>
        {route.kind === "articles" && <ArticleList profileId={selectedProfile.id} filter={route.filter} onOpen={(id) => setRoute({ kind: "article", id })} onGoVoices={() => setRoute({ kind: "voices" })} onGoTopics={() => setRoute({ kind: "topics" })} onMainPrimary={setMainPrimary} onShowEngineSteps={onShowEngineSteps} />}
        {route.kind === "topics" && <TopicPicker onBack={() => setRoute({ kind: "articles", filter: "all" })} onQueued={() => setRoute({ kind: "articles", filter: "all" })} onGoVoices={() => setRoute({ kind: "voices" })} />}
        {route.kind === "article" && <Editor key={`${selectedProfile.id}:${route.id}`} profileId={selectedProfile.id} id={route.id} onBack={() => setRoute({ kind: "articles", filter: "all" })} onDeleted={() => setRoute({ kind: "articles", filter: "all" })} onGoVoices={() => setRoute({ kind: "voices" })} onAdjustVoice={(voiceId) => setRoute({ kind: "voices", voiceId, backTo: { kind: "article", id: route.id } })} onRegisterFlush={registerEditorFlush} profileChanging={profileChanging} />}
        {route.kind === "voices" && <VoiceSection profileId={selectedProfile.id} voiceId={route.voiceId} onBackToArticle={route.backTo ? () => setRoute(route.backTo!) : undefined} />}
      </main>
    </div>
  );
}
