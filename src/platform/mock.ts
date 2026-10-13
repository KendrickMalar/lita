// An in-browser stand-in for the native side, used only when the app is
// built with VITE_LITA_MOCK=1 (see `npm run mock`). It lets every screen be
// opened in an ordinary browser, which is how design reviews take their
// screenshots. The scene comes from the URL: `?scene=voice`, `?scene=write`…
//
// Scenes: launch (waiting on the sign-in), launch-slow (waiting on Codex,
// past three seconds), signed-out, signing-in, codex-not-logged-in, codex-not-installed,
// intake, intake-open (note row unfolded), intake-connecting (note import
// running, never finishing), intake-loaded, intake-manual (three pieces
// pasted by hand), intake-both (one pasted piece plus a note import),
// building, voice,
// voice-open (same as voice: the eight items are no longer folded),
// voice-evidence (one row's quotes already open),
// intake-preset (the intake with the road to the voice that comes with
// Lita in view; the same screen as intake, kept as its own name so the
// screenshot has one), voice-preset (that voice, just made: no writing
// behind it yet),
// voice-sources (mixed connections), voice-template (older
// profile without one_line), voices (two voices), articles-first-run,
// update-available, update-downloading, update-latest, articles, articles-empty, editor, editor-empty,
// editor-generating, editor-over, editor-save-failed, editor-versions, editor-note,
// editor-brief-suggesting (Lita is working out what to write),
// editor-brief-suggested (the suggestion has just landed, with a way back),
// editor-generating-long (writing a note article, where the wait is minutes),
// write, write-generating,
// write-result, write-over, write-error,
// topics (picking titles, policy empty, the cloud already gathered),
// topics-picked (two words pressed, ten titles, three picked, policy filled),
// topics-picked-few (ten titles, three picked, no cloud to press),
// topics-policy-open (the policy opened, all three lines empty),
// topics-policy-open-filled (the policy opened, filled),
// topics-cloud-first (no cloud yet, gathering it on open, never finishing),
// topics-cloud-picked (three words pressed), topics-cloud-more (more
// material since the cloud was gathered), topics-cloud-few (too few words
// to make a cloud: one sentence and "add writing" in its place), topics-cloud-three
// (three words, the fewest that still make a cloud), topics-cloud-few-more
// (too few words, and writing added since: gathered again on opening),
// topics-cloud-failed (gathering did not work), topics-cloud-stopped (writing,
// no cloud, the first gather stopped), topics-add-empty (only the voice that
// comes with Lita: no writing, no articles), topics-add-open (the same, with
// "add writing" opened; adding gathers the words), topics-add-failed (the
// same, and adding does not work), topics-add-articles (articles, but no
// writing of one's own), topics-add-unlisted (the same as topics-add-empty,
// but the voices cannot be listed, so there is nowhere to add to),
// topics-cloud-unread (the saved cloud cannot be read),
// topics-add-talk (add writing opened, a conversation pasted, the name
// already remembered: only that person's messages are in the field),
// topics-add-talk-who (the same conversation, no name remembered: which one
// is you), topics-add-talk-dup (one of your messages was added before),
// topics-add-talk-none (you are there, but with nothing left to keep),
// topics-add-talk-unsure (times recur, names cannot be told apart),
// voice-sources-talk (the voice page with two rows added from conversations),
// articles-queued (one writing, two waiting),
// articles-queue-failed (one written, one not written, one waiting, and the
// whole thing stopped), profiles-menu (two profiles, the account menu open),
// profiles-create (two profiles, the new-profile name form open). Any scene
// whose name contains "profiles" has a second profile.
// The engine scenes (Codex or Claude Code writes). Every scene above has Codex
// chosen and both engines ready, except codex-not-logged-in and
// codex-not-installed, which change only Codex's state. The ones below start
// with the other choices:
// engine-menu (the body, account menu open; Codex chosen, both ready),
// engine-menu-claude (the same; Claude Code chosen and ready),
// engine-menu-claude-not-logged-in (the same; Claude Code chosen, not logged in:
// the way to fix it in the menu, and the warning at the bottom left),
// engine-menu-claude-not-installed (the same; Claude Code chosen, not installed),
// engine-steps-claude-not-logged-in (the steps screen: Claude Code chosen, not
// logged in, Codex ready; "Use Codex" there changes the choice and opens the body),
// engine-steps-claude-not-installed (the same; Claude Code not installed),
// articles-queue-stopped-claude (Claude Code chosen and ready at launch; the list
// is the stopped one of articles-queue-failed, stopped on claude_not_logged_in).
// Add `&lang=en` to force English.
// `&delay=<ms>` slows every read (the lists, one article, one voice), so the
// loading rules can be watched: under 300 ms nothing is said at all.

import type * as T from "./types";

const params = new URLSearchParams(typeof location === "undefined" ? "" : location.search);
export const scene = params.get("scene") ?? "voice";

/** The mock's opinion of how a screen should start; null in real builds. */
export function mockScene(): string | null {
  return import.meta.env.VITE_LITA_MOCK ? scene : null;
}

const never = <T,>() => new Promise<T>(() => {});
const noteName = scene === "voices" ? "note・kuno" : "test";
const wait = (ms: number) => (ms > 0 ? new Promise<void>((r) => setTimeout(r, ms)) : Promise.resolve());

/** `?delay=800` makes every read that slow, to watch the loading rules. */
const readDelay = Math.max(0, Number(params.get("delay") ?? "0")) || 0;

const profile: T.VoiceProfile = {
  language: "ja",
  first_person: "私",
  formality: "常体",
  tone: ["分析的", "懐疑的"],
  sentence_endings: ["のだろうか。", "と思う。", "ではないか。"],
  avg_sentence_length_chars: 42,
  preferred_words: ["エクイティ", "ファイナンス", "構造"],
  avoided_words: ["絶対", "最強"],
  opens_with: "問いから入る",
  closes_with: "含みを残して締める",
  uses_emoji: false,
  representative_excerpts: [
    { excerpt: "資本政策は、何を諦めるかを先に決める作業なのだろうか。", why: "問いで締める癖が出ている" },
    { excerpt: "ファイナンスの話は結局、時間をどう買うかに帰着すると思う。", why: "抽象化して言い切る" },
  ],
  one_line: scene === "voice-template" ? "" : "あなたの文章は、常体で分析的。エクイティやファイナンスを軸に問いを立て、「のだろうか」と含みを残して締める。",
};

// The stricter reading (2026-09-23): quotes behind each item, and how sure it
// is. `voice-template` keeps the old shape, so the screen shows nothing extra.
const strict: Partial<T.VoiceProfile> = {
  rhetoric: "言い直して畳みかけ、最後に逆接を一つ置く",
  examples_and_numbers: "自分が立ち会った場面を出し、数字は概数で置く",
  kana_choices: ["できる", "こと", "ほとんど"],
  never_does: ["感嘆符を使わない", "読者に呼びかけない", "体言止めをしない"],
  word_usage: [
    { word: "エクイティ", usage: "「借りる」と対にして、返さない代わりに何を渡すかの話に使う" },
    { word: "構造", usage: "個人の努力の話を、仕組みの話に引き取るときに使う" },
  ],
  topic_words: ["資本政策", "投資家"],
  backing: {
    preferred_words: { confidence: "high", evidence: [
      { piece: 0, quote: "エクイティは返さなくていい金ではなく、時間と自由を先に売る契約だ。" },
      { piece: 1, quote: "ファイナンスの話は結局、時間をどう買うかに帰着すると思う。" },
      { piece: 2, quote: "努力の差に見えるものは、たいてい構造の差だ。" },
    ] },
    avoided_words: { confidence: "low", evidence: [
      { piece: 0, quote: "絶対に正しい資本政策というものは、たぶん無い。" },
    ] },
    first_person: { confidence: "high", evidence: [
      { piece: 0, quote: "私はこの質問を、相談の最初に必ず置く。" },
      { piece: 3, quote: "私が見てきた範囲では、ここで迷う人がいちばん多い。" },
    ] },
    formality: { confidence: "high", evidence: [
      { piece: 1, quote: "資本政策は、何を諦めるかを先に決める作業だ。" },
    ] },
    tone: { confidence: "medium", evidence: [
      { piece: 2, quote: "その数字は、本当に比べられるものを比べているのだろうか。" },
    ] },
    sentence_endings: { confidence: "high", evidence: [
      { piece: 0, quote: "迷っている理由の方が本題ではないか。" },
      { piece: 1, quote: "時間をどう買うかに帰着すると思う。" },
    ] },
    opens_with: { confidence: "medium", evidence: [
      { piece: 3, quote: "いくら欲しいか、と聞かれて答えられる人は少ない。" },
    ] },
    closes_with: { confidence: "medium", evidence: [
      { piece: 0, quote: "先に売る契約なのだろうか。" },
    ] },
    rhetoric: { confidence: "medium", evidence: [
      { piece: 2, quote: "順番の問題だ。順番だけの問題だ。それでも順番は決まらない。" },
    ] },
    examples_and_numbers: { confidence: "medium", evidence: [
      { piece: 3, quote: "十人ほど見てきたが、最初の一回で決められた人はいない。" },
    ] },
    kana_choices: { confidence: "high", evidence: [
      { piece: 1, quote: "ここで決めることは、ほとんど決まっていると思う。" },
    ] },
    never_does: { confidence: "medium", evidence: [
      { piece: 2, quote: "答えはほとんど決まっていると思う。" },
    ] },
  },
};

// The voice that comes with Lita (D-70): written by hand, copied as it is.
// No backing, no quotes, no measurements — there is no writing behind it,
// and that is a normal state, not a fault.
const PRESET_ONE_LINE = "あなたの文章は、です・ます調で平易に、要点を先に言い、読者への一つのお願いで静かに締めます。";
const presetProfile: T.VoiceProfile = {
  language: "ja",
  first_person: "私たち",
  formality: "です・ます調",
  tone: ["丁寧", "平易"],
  sentence_endings: ["です。", "ます。", "ません。"],
  avg_sentence_length_chars: 38,
  preferred_words: ["お知らせします", "ご案内します", "たとえば"],
  avoided_words: ["弊社", "させていただく", "業界初"],
  opens_with: "要点を先に一文で言う",
  closes_with: "読者への一つのお願いで締める",
  uses_emoji: false,
  representative_excerpts: [],
  one_line: PRESET_ONE_LINE,
};
const presetVoice: T.Voice = {
  id: "v-preset", name: "広報のです・ます", profile: presetProfile,
  created_at: "2026-09-24T02:00:00Z", updated_at: "2026-09-24T02:00:00Z", voice_sources: [],
};

const sources: T.VoiceSource[] = [1, 2, 3, 4].map((i) => ({
  id: `s${i}`, kind: "note", origin: `https://note.com/kuno/n/n${i}`, account: "kuno",
  body: "資本政策は、何を諦めるかを先に決める作業なのだろうか。".repeat(8), created_at: "2026-09-22T09:00:00Z",
}));

const voice2: T.Voice = {
  id: "v2", name: "貼った文章", profile: { ...profile, formality: "です・ます調", tone: ["落ち着いた", "丁寧"], sentence_endings: ["と思います。", "ではないでしょうか。"], one_line: "あなたの文章は、です・ます調で落ち着いて丁寧。「ではないでしょうか」と問いかけて締める。" },
  created_at: "2026-09-23T01:00:00Z", updated_at: "2026-09-23T01:00:00Z", voice_sources: sources.slice(0, 2),
};
const handBodies = [
  "エクイティは返さなくていい金ではなく、いちばん高い金です。返済期限がないぶん、会社の持ち分を削ります。",
  "買収の相談で最初に聞くのは値段ではありません。売り手が何を手放したくないか、です。",
  "小さな会社の資金繰りは、月末ではなく週で見ます。遅れている入金と、動かせる支払いが分けて見えます。",
];
const mixedSources: T.VoiceSource[] = [
  ...sources,
  { id: "m1", kind: "medium", origin: "https://medium.com/@kuno/a", account: "kuno", body: "…", created_at: "2026-09-23T02:00:00Z" },
  { id: "m2", kind: "medium", origin: "https://medium.com/@kuno/b", account: "kuno", body: "…", created_at: "2026-09-23T02:00:00Z" },
  ...handBodies.map((body, i) => ({ id: `p${i + 1}`, kind: "paste" as const, origin: null, account: null, body, created_at: "2026-09-23T02:30:00Z" })),
];
let voice: T.Voice = {
  id: "v1", name: noteName, profile: scene === "voice-template" ? profile : { ...profile, ...strict }, created_at: "2026-09-22T09:00:00Z", updated_at: "2026-09-22T09:00:00Z", voice_sources: scene === "voice-sources" ? mixedSources : sources,
};

if (["voice-preset", "topics-add-empty", "topics-add-open", "topics-add-failed", "topics-add-unlisted"].includes(scene) || scene.startsWith("topics-add-talk")) voice = presetVoice;

// ----- a pasted conversation (D-77). Every name and line is invented. -----
// The browser has no Rust, so the reading of the one sample below is written
// out by hand; anything else pasted in the mock is read as ordinary writing.
const talkMe = "森川 陽介";
const talkMessages: T.TalkMessage[] = [
  { speaker: "佐藤 花子", body: "来週の採用面談、評価シートはどこに置いてありますか？", seen: false },
  { speaker: talkMe, body: "共有ドライブの「採用」フォルダに置きました。見てほしいのは点数ではなく、面談官ごとのばらつきです。", seen: false },
  { speaker: "Mika Tanaka", body: "I'll join the Tuesday one.", seen: false },
  { speaker: talkMe, body: "資金繰り表は週次に切り替えました。月末だけ見ていると、遅れている入金に気づくのが遅れます。", seen: false },
  { speaker: talkMe, body: "撤退基準、始める前に数字で決めておきませんか。始めてからだと、どうしても甘くなります。", seen: false },
];
const talkSample = "佐藤 花子  10:21\n来週の採用面談、評価シートはどこに置いてありますか？\n森川 陽介  10:23\n…";
const unsureSample = "10:21\n来週の採用面談、評価シートを先に共有しておきます。\n\n10:23\n資金繰り表は週次に切り替えました。";
if (scene === "topics-add-talk-dup") {
  voice = { ...voice, voice_sources: [{ id: "t0", kind: "paste", origin: "talk:2026-09-24", account: null, body: talkMessages[1].body, created_at: "2026-09-24T03:00:00Z" }] };
}
if (scene === "voice-sources-talk") {
  voice = { ...voice, voice_sources: [
    ...handBodies.slice(0, 2).map((body, i) => ({ id: `p${i + 1}`, kind: "paste" as const, origin: null, account: null, body, created_at: "2026-09-23T02:30:00Z" })),
    { id: "t1", kind: "paste", origin: "talk:2026-09-24", account: null, body: talkMessages[1].body, created_at: "2026-09-24T03:00:00Z" },
    { id: "t2", kind: "paste", origin: "talk:2026-09-26", account: null, body: `${talkMessages[3].body}\n\n${talkMessages[4].body}`, created_at: "2026-09-26T03:00:00Z" },
    ...sources,
  ] };
}

/** What the talk scenes paste as the field opens; null elsewhere and in real builds. */
export function mockTalkPaste(): string | null {
  if (!mockScene()?.startsWith("topics-add-talk")) return null;
  return scene === "topics-add-talk-unsure" ? unsureSample : talkSample;
}

/** The names this device remembers in the talk scenes; null leaves it to the real store. */
export function mockTalkMe(): string[] | null {
  if (!mockScene()?.startsWith("topics-add-talk")) return null;
  return scene === "topics-add-talk-who" ? [] : [talkMe];
}

function mockReadTalk(text: string, known: string[]): T.TalkReading {
  if (text === unsureSample) return { kind: "unsure" };
  if (!text.startsWith(talkSample)) return { kind: "plain" };
  const key = (s: string) => s.replace(/\s+/g, "");
  const seen = new Set(known.flatMap((k) => k.split(/\n\s*\n/)).map(key));
  // "none": you are in the conversation, but only with a sticker.
  const messages = scene === "topics-add-talk-none" ? talkMessages.filter((m) => m.speaker !== talkMe) : talkMessages;
  return { kind: "talk", speakers: ["佐藤 花子", talkMe, "Mika Tanaka"], messages: messages.map((m) => ({ ...m, seen: seen.has(key(m.body)) })) };
}
// Lita wrote the articles; none of the writing is the person's own.
if (scene === "topics-add-articles") voice = { ...voice, voice_sources: [] };
let personalVoice: T.Voice = { ...voice, id: "bv1", name: "個人発信の文体", profile: { ...voice.profile, one_line: "自分の経験から考えを組み立てる、個人発信の文体。" }, voice_sources: sources.slice(0, 2) };

const pieces: T.Piece[] = Array.from({ length: 5 }, (_, i) => ({
  title: ["資本政策は諦める順番を決める作業", "ファイナンスは時間を買う話", "エクイティの重さについて", "投資家との最初の会話で聞くこと", "利益率の議論が空回りする理由"][i],
  url: `https://note.com/kuno/n/n${i + 1}`, published_at: "2026-09-01",
  text: "資本政策は、何を諦めるかを先に決める作業なのだろうか。ファイナンスの話は結局、時間をどう買うかに帰着すると思う。".repeat(6 + i),
}));

const draftBody = "資本政策の相談で最初に聞くのは、いくら欲しいかではなく、何を諦められるかだ。エクイティは返さなくていい金ではなく、時間と自由を先に売る契約なのだろうか。";
const longBody = draftBody + "資本政策の話は、結局のところ何を諦めるかの順番を決める作業でしかないのだろうか。" + "ファイナンスの選択肢を並べる前に、まず自分が手放せないものを一つ決める。そこから逆算すると、借入か出資かの答えはほとんど決まっていると思う。それでも迷うなら、迷っている理由の方が本題ではないか。" + "ファイナンスの選択肢を並べる前に、まず自分が手放せないものを一つ決める。そこから逆算すると、借入か出資かの答えはほとんど決まっていると思う。それでも迷うなら、迷っている理由の方が本題ではないか。";

// The engine scenes. Plain objects and exact names only: this file is also
// loaded with its types stripped, so nothing here may need a compiler.
/** Which engine each engine scene starts with; every other scene starts with Codex. */
const sceneEngine: Record<string, T.EngineKind> = {
  "engine-menu": "codex",
  "engine-menu-claude": "claude",
  "engine-menu-claude-not-logged-in": "claude",
  "engine-menu-claude-not-installed": "claude",
  "engine-steps-claude-not-logged-in": "claude",
  "engine-steps-claude-not-installed": "claude",
  "articles-queue-stopped-claude": "claude",
};
const claudeNotLoggedInScenes = ["engine-menu-claude-not-logged-in", "engine-steps-claude-not-logged-in"];
const claudeNotInstalledScenes = ["engine-menu-claude-not-installed", "engine-steps-claude-not-installed"];
const queueStoppedScenes = ["articles-queue-stopped-claude"];

const codex: T.EngineStatus =
  scene === "codex-not-logged-in" ? { status: "not_logged_in", version: "0.154.0" }
  : scene === "codex-not-installed" ? { status: "not_installed" }
  : { status: "ready", version: "0.154.0" };
const claude: T.EngineStatus =
  claudeNotLoggedInScenes.includes(scene) ? { status: "not_logged_in", version: "2.1.296" }
  : claudeNotInstalledScenes.includes(scene) ? { status: "not_installed" }
  : { status: "ready", version: "2.1.296" };
/** The engine in use; `selectEngine` changes this and nothing else. */
let mockEngine: T.EngineKind = sceneEngine[scene] ?? "codex";

const session: T.SessionStatus =
  scene === "signed-out" ? { status: "signed_out" } : { status: "signed_in", email: "kuno@muumoo.online" };

const initialProfile: T.PublishingProfile = { id: "profile-initial", name: "プロフィール 1", is_initial: true, created_at: "2026-09-23T00:00:00Z" };
const secondProfile: T.PublishingProfile = { id: "profile-personal", name: "個人発信", is_initial: false, created_at: "2026-09-27T00:00:00Z" };
const hasSecondProfile = scene.includes("profiles");
let mockProfiles: T.PublishingProfile[] = [initialProfile, ...(hasSecondProfile ? [secondProfile] : [])];
let selectedProfileId = initialProfile.id;
let nextProfileId = 1;
let hasVoice = !["intake", "intake-preset", "intake-open", "intake-connecting", "intake-loaded", "intake-manual", "intake-both", "building", "articles-first-run", "articles-no-voice", "topics-no-voice"].includes(scene);
const cloneVoice = (value: T.Voice): T.Voice => ({
  ...value,
  profile: { ...value.profile },
  voice_sources: value.voice_sources.map((source) => ({ ...source })),
});
const profileVoices = new Map<string, T.Voice[]>([
  [initialProfile.id, hasVoice ? [cloneVoice(voice), ...(scene.startsWith("voices") ? [cloneVoice(voice2)] : [])] : []],
  [secondProfile.id, hasSecondProfile && hasVoice ? [cloneVoice(personalVoice)] : []],
 ]);
const currentVoices = (profileId = selectedProfileId) => profileVoices.get(profileId) ?? [];
const findCurrentVoice = (id: string) => currentVoices().find((item) => item.id === id) ?? null;
const updateCurrentVoice = (id: string, update: (value: T.Voice) => T.Voice) => {
  const voices = currentVoices();
  const index = voices.findIndex((item) => item.id === id);
  if (index < 0) return null;
  const updated = update(voices[index]);
  profileVoices.set(selectedProfileId, voices.map((item, currentIndex) => currentIndex === index ? updated : item));
  return updated;
};

// ----- articles (in-memory) -------------------------------------------------

const now = new Date();
const ago = (mins: number) => new Date(now.getTime() - mins * 60000).toISOString();
const articleBodies = [
  { title: "資本政策は諦める順番を決める作業", body: draftBody, brief: "資本政策の相談を受けたときに最初に聞くことについて。創業者に向けて、エクイティは時間を売る契約だと伝えたい。", platform: "x", status: "approved", at: ago(35) },
  { title: scene.startsWith("editor-brief") ? "利益率の議論が空回りする理由" : "", body: scene === "editor-over" ? longBody : (scene === "editor-empty" ? "" : draftBody), brief: scene === "editor-empty" ? "" : (scene.startsWith("editor-brief") ? "利益率の議論が空回りする理由について、経営者向けに。分母を揃える話を一つだけ。\n長さは 1,500〜3,000 字。\n外部の事実は調べず、自分の経験と意見の範囲で書く。" : "利益率の議論が空回りする理由について、経営者向けに。分母を揃える話を一つだけ。"), platform: "x", status: "draft", at: ago(3) },
  { title: "投資家との最初の会話で聞くこと", body: "投資家との最初の会話で聞くべきことは一つで、彼らが何を恐れているかだ。\n\nリターンの話は後からいくらでもできる。恐れが分かれば、こちらの提案の形はほとんど決まる。\n\n## 恐れは三つに分かれる\n\n一つ目は時間、二つ目は評判、三つ目は次の資金調達だ。", brief: "投資家との初回面談で何を聞くべきか。note 向けに 1,500 字ほど。", platform: "note", status: "draft", at: ago(60 * 26) },
  { title: "ファイナンスは時間を買う話", body: draftBody, brief: "ファイナンスの選択肢の話。", platform: "x", status: "archived", at: ago(60 * 24 * 4) },
];
const articles: T.Article[] = ["articles-empty", "articles-first-run", "topics-add-empty", "topics-add-open", "topics-add-failed", "topics-add-unlisted"].includes(scene) ? [] : articleBodies.map((a, i) => ({
  id: `a${i + 1}`, voice_id: "v1", platform_id: a.platform, title: a.title, body: a.body, brief: a.brief,
  status: a.status as T.ArticleStatus, queue: null, created_at: a.at, updated_at: a.at,
}));
// The editor scenes open a2 (a draft with a brief).
if (scene.startsWith("editor")) { const a = articles.find((x) => x.id === (scene === "editor-note" || scene === "editor-generating-long" ? "a3" : "a2")); if (a) { articles.splice(articles.indexOf(a), 1); articles.unshift({ ...a, id: "a1" }); articles[1] = { ...articles[1], id: "a2" }; } }

// ----- the queue (article-queue, 2026-09-23) --------------------------------

// The two article-list scenes hold still so a screenshot catches them; the
// queue only really moves when titles are lined up in this session.
const frozen = scene === "articles-queued" || scene === "articles-queue-failed" || queueStoppedScenes.includes(scene);
const queuedTitles = ["借入とエクイティ、どちらが高い金か", "資金繰りは月ではなく週で見る", "撤退の基準は、始める前に決める"];
if (frozen) {
  const failed = scene === "articles-queue-failed" || queueStoppedScenes.includes(scene);
  const states: (T.QueueState | null)[] = failed ? [null, "failed", "waiting"] : ["writing", "waiting", "waiting"];
  articles.unshift(...queuedTitles.map((title, i) => ({
    id: `q${i + 1}`, voice_id: "v1", platform_id: "note", title,
    body: failed && i === 0 ? "返さなくていい金が、いちばん高い金になることがあります。" : "",
    brief: "返さない金の方が高くつく、という話を一本。", status: "draft" as T.ArticleStatus,
    queue: states[i], created_at: ago(i), updated_at: ago(i),
  })));
}
if (hasSecondProfile) {
  articles.unshift(
    { id: "aq1", voice_id: "v1", platform_id: "note", title: "会社広報の順番待ち", body: "", brief: "会社向けの話を書く。", status: "draft", queue: "waiting", created_at: ago(1), updated_at: ago(1) },
    { id: "aq0", voice_id: "v1", platform_id: "note", title: "会社広報が書いています", body: "", brief: "会社向けの話を書く。", status: "draft", queue: "writing", created_at: ago(2), updated_at: ago(2) },
  );
}
const personalArticles: T.Article[] = hasSecondProfile ? [
  { id: "bp1", voice_id: "bv1", platform_id: "x", title: "個人発信の下書き", body: "自分の視点で書いた別プロフィールの記事です。", brief: "個人の発信について。", status: "draft", queue: "writing", created_at: ago(2), updated_at: ago(2) },
  { id: "bp2", voice_id: "bv1", platform_id: "note", title: "個人発信の順番待ち", body: "", brief: "個人の話を書く。", status: "draft", queue: "waiting", created_at: ago(1), updated_at: ago(1) },
 ] : [];
const articlesByProfile = new Map<string, T.Article[]>([[initialProfile.id, articles]]);
if (hasSecondProfile) articlesByProfile.set(secondProfile.id, personalArticles);
const currentArticles = () => articlesByProfile.get(selectedProfileId) ?? [];

const emptyPolicy: T.Policy = { audience: "", takeaway: "", topics: [], avoid: "" };
const fullPolicy: T.Policy = {
  audience: "これから会社を買う経営者",
  takeaway: "値段より先に見るものがある",
  topics: ["資金繰り", "買収", "採用"],
  avoid: "個別の会社名",
};
let policy: T.Policy = ["topics-picked", "topics-cloud-picked", "topics-policy-open-filled"].includes(scene) ? fullPolicy : emptyPolicy;
const profilePolicies = new Map<string, T.Policy>([
  [initialProfile.id, policy],
  [secondProfile.id, { audience: "個人の読者", takeaway: "自分の経験から考える", topics: [], avoid: "会社の内部情報" }],
 ]);
const currentPolicy = () => profilePolicies.get(selectedProfileId) ?? emptyPolicy;

// The words someone keeps writing about (2026-09-24). `weight` is 1–5 and is
// folded into three sizes on screen; `written` marks a subject already used
// as a title. Thirty-six words, near the thirty the gatherer aims for.
const cloudWords: T.CloudWord[] = ([
  ["在庫", 1, false], ["契約書", 2, false], ["顧問", 1, true], ["粗利", 3, false], ["月次", 2, false],
  ["人件費", 3, false], ["事業承継", 3, false], ["銀行", 3, false], ["資金繰り", 5, false],
  ["値付け", 3, false], ["のれん", 3, true], ["買収", 5, false], ["借入", 3, false],
  ["売り手", 3, false], ["数字の読み方", 3, false], ["採用", 5, false], ["権限委譲", 3, false],
  ["撤退基準", 3, false], ["資本政策", 3, true], ["現場", 3, false], ["定着", 3, false],
  ["社長の時間", 3, false], ["評価", 3, false], ["投資家", 3, true], ["会議", 1, false],
  ["経営計画", 3, false], ["引き継ぎ", 2, false], ["税務", 1, false], ["キャッシュ", 3, false],
  ["組織", 3, false], ["給与", 2, false], ["独立", 1, false], ["小さな会社", 4, false],
  ["業界構造", 2, false], ["デューデリジェンス", 2, false], ["ミドルマネジメント", 1, false],
] as [string, number, boolean][]).map(([word, weight, written]) => ({ word, weight, written }));

const fullCloud: T.TopicCloud = { words: cloudWords, gathered_at: ago(2), material_count: 18 };
// Under three words is not a cloud, and nothing stands in its place.
const sparseCloud: T.TopicCloud = { words: cloudWords.slice(0, 2), gathered_at: ago(2), material_count: 2 };
const threeCloud: T.TopicCloud = { words: [cloudWords[8], cloudWords[6], cloudWords[0]], gathered_at: ago(2), material_count: 2 };

const cloudView = (): T.CloudView => {
  if (!scene.startsWith("topics")) return { cloud: null, material_count: 0 };
  if (scene === "topics-cloud-first" || scene === "topics-cloud-stopped") return { cloud: null, material_count: 18 };
  // Nothing of one's own yet: the count is the writing added in this session.
  if (scene.startsWith("topics-add")) return { cloud: null, material_count: currentVoices(initialProfile.id).reduce((count, item) => count + item.voice_sources.length, 0) };
  if (scene === "topics-cloud-few" || scene === "topics-picked-few") return { cloud: sparseCloud, material_count: 2 };
  if (scene === "topics-cloud-three") return { cloud: threeCloud, material_count: 2 };
  if (scene === "topics-cloud-few-more") return { cloud: sparseCloud, material_count: 4 };
  if (scene === "topics-cloud-failed") return { cloud: null, material_count: 18 };
  // More material than the cloud was gathered from: one quiet line offers to
  // gather again, and never says how many more.
  if (scene === "topics-cloud-more") return { cloud: fullCloud, material_count: 24 };
  return { cloud: fullCloud, material_count: 18 };
};
const personalCloud: T.TopicCloud = {
  words: [
    { word: "働き方", weight: 4, written: false },
    { word: "独立", weight: 3, written: false },
    { word: "学び", weight: 2, written: false },
  ],
  gathered_at: ago(2),
  material_count: 2,
};
const gatheredCloudsByProfile = new Map<string, T.TopicCloud>();
const emptyCloudView: T.CloudView = { cloud: null, material_count: 0 };
const sourceCount = (profileId: string) => currentVoices(profileId).reduce((count, item) => count + item.voice_sources.length, 0);
const currentCloudView = (profileId = selectedProfileId): T.CloudView => {
  const gathered = gatheredCloudsByProfile.get(profileId);
  if (gathered) {
    const currentCount = profileId === initialProfile.id && !scene.startsWith("topics-add") ? cloudView().material_count : sourceCount(profileId);
    return { cloud: { ...gathered, words: gathered.words.map((word) => ({ ...word })) }, material_count: currentCount };
  }
  if (profileId === initialProfile.id) {
    const view = cloudView();
    return { ...view, material_count: scene.startsWith("topics-add") ? sourceCount(profileId) : view.material_count };
  }
  if (profileId === secondProfile.id && hasSecondProfile) {
    return { cloud: { ...personalCloud, words: personalCloud.words.map((word) => ({ ...word })) }, material_count: sourceCount(profileId) };
  }
  return emptyCloudView;
};
const cloudForProfile = (profileId: string): T.TopicCloud | null => {
  if (profileId === initialProfile.id) return cloudView().cloud ?? fullCloud;
  if (profileId === secondProfile.id && hasSecondProfile) return personalCloud;
  return null;
};

const topicRounds = [
  [
    "借入とエクイティ、どちらが高い金か",
    "値付けの前に、売り手が手放したくないもの",
    "資金繰りは月ではなく週で見る",
    "小さな会社の採用は、席ではなく仕事で決める",
    "当たらない前提で事業計画を作る",
    "のれんの話を、経営の言葉に直す",
    "社長が数字を読めるようになる順番",
    "撤退の基準は、始める前に決める",
    "銀行との面談で、先に出す一枚",
    "買収の後、最初の 90 日でしないこと",
  ],
  [
    "引き継ぎの三か月で、先に壊すもの",
    "月次が出るのが遅い会社に共通すること",
    "面接で聞かない方がいい質問",
    "値引きを断る言い方を先に決めておく",
    "社長が現場を離れる順番",
    "在庫は、決算より先に人を縛る",
    "digital でない会社の digital 化",
    "報酬の決め方を、先に紙に書く",
    "仕入先を一社に寄せたときに起きたこと",
    "辞めた人の穴を、採用で埋めない",
  ],
];
const topicRoundsByProfile = new Map<string, number>();

// What ten titles look like once words have been pressed: every one of them
// is about the subjects that were picked. Money and exit words get their own ten.
const cashTitles = [
  "資金繰りは月ではなく週で見る",
  "入金が遅れたときに、先に電話する相手",
  "撤退の基準は、始める前に決める",
  "支払いを待ってもらう頼み方",
  "赤字の事業を、いつまで続けるか",
  "借入の枠は、困る前に作っておく",
  "やめる判断を、数字だけに任せない",
  "手元の現金が何か月もつかを、毎週書く",
  "撤退を決めた後、最初に伝える相手",
  "資金繰り表は、社長が自分で書く",
];
const subjectTitles = [
  "小さな会社の採用は、席ではなく仕事で決める",
  "入社 3 か月で辞める人と、辞めない人の差",
  "求人票に書けないことを、面接で先に言う",
  "評価は、上げる理由より下げない理由で書く",
  "採用の前に、いまの人の仕事を一つ減らす",
  "給与を上げる前に、決める順番がある",
  "定着しない職場に共通する、朝の 10 分",
  "未経験を採るなら、教える人の時間を先に空ける",
  "社長が面接に出るのをやめる日",
  "辞めた人の理由は、辞める前に聞ける",
];

const queueHandlers = new Set<(e: T.QueueEvent) => void>();
const emitQueue = (e: T.QueueEvent) => queueHandlers.forEach((h) => h(e));
const queueTimers = new Map<string, number>();

function stepQueue(profileId: string = selectedProfileId) {
  if (frozen || queueTimers.has(profileId)) return;
  const rows = articlesByProfile.get(profileId) ?? [];
  const target = rows.find((article) => article.queue === "writing") ?? rows.find((article) => article.queue === "waiting");
  if (!target) return;
  target.queue = "writing";
  emitQueue({ kind: "changed" });
  const timer = window.setTimeout(() => {
    queueTimers.delete(profileId);
    target.body = longBody;
    target.queue = null;
    target.updated_at = new Date().toISOString();
    emitQueue({ kind: "changed" });
    stepQueue(profileId);
  }, 3000);
  queueTimers.set(profileId, timer);
}

const versions: T.ArticleVersion[] = scene === "editor-versions" ? [
  { id: "v3", article_id: "a1", kind: "edited", title: "", body: draftBody, prompt_sent: null, elapsed_ms: null, created_at: ago(3) },
  { id: "v2", article_id: "a1", kind: "shortened", title: "", body: draftBody, prompt_sent: "…", elapsed_ms: 6000, created_at: ago(25) },
  { id: "v1", article_id: "a1", kind: "generated", title: "", body: draftBody + "資本政策の話は、結局のところ何を諦めるかの順番を決める作業でしかないのだろうか。", prompt_sent: "…", elapsed_ms: 6600, created_at: ago(40) },
] : [];
const versionsByProfile = new Map<string, T.ArticleVersion[]>([[initialProfile.id, versions]]);
if (hasSecondProfile) versionsByProfile.set(secondProfile.id, []);
const currentVersions = () => versionsByProfile.get(selectedProfileId) ?? [];
let nextId = 100;
const excerpt = (b: string) => (b.trim().split("\n").find((l) => l.trim()) ?? "").slice(0, 80);

// Listeners for import-progress, as the native side has; importNote feeds them.
const importHandlers = new Set<(p: T.ImportProgress) => void>();
const emitImport = (p: T.ImportProgress) => importHandlers.forEach((h) => h(p));

export const mockHost: T.Host = {
  listArticles: async (status) => { await wait(readDelay); return currentArticles().filter((a) => !status || a.status === status).map(({ body, ...a }) => ({ ...a, excerpt: excerpt(body) })); },
  getArticle: async (id) => { await wait(readDelay); return currentArticles().find((a) => a.id === id) ?? null; },
  updateArticle: async (id, patch) => {
    if (scene.includes("save-failed")) throw { code: "network", detail: "fetch failed: network unavailable" };
    const a = currentArticles().find((x) => x.id === id); if (a) Object.assign(a, patch, { updated_at: new Date().toISOString() });
  },
  deleteArticle: async (id) => { const rows = currentArticles(); const i = rows.findIndex((a) => a.id === id); if (i >= 0) rows.splice(i, 1); return i >= 0; },
  listVersions: async (articleId) => currentVersions().filter((version) => version.article_id === articleId),
  snapshotArticle: async (articleId, manual) => {
    const a = currentArticles().find((x) => x.id === articleId); if (!a) return null;
    const v: T.ArticleVersion = { id: `v${nextId++}`, article_id: articleId, kind: manual ? "manual" : "edited", title: a.title, body: a.body, prompt_sent: null, elapsed_ms: null, created_at: new Date().toISOString() };
    currentVersions().unshift(v); return v;
  },
  restoreVersion: async (versionId) => {
    const v = currentVersions().find((x) => x.id === versionId)!; const a = currentArticles().find((x) => x.id === v.article_id)!;
    Object.assign(a, { title: v.title, body: v.body }); return a;
  },
  generateIntoArticle: async (articleId, _effort, previous) => {
    if (scene === "editor-generating" || scene === "editor-generating-long") return never();
    await wait(400);
    const a = currentArticles().find((x) => x.id === articleId)!;
    a.body = previous ? draftBody : (a.platform_id === "x" ? draftBody : longBody);
    if (a.platform_id !== "x" && !a.title) a.title = "資本政策は諦める順番を決める作業";
    const v: T.ArticleVersion = { id: `v${nextId++}`, article_id: articleId, kind: previous ? "shortened" : "generated", title: a.title, body: a.body, prompt_sent: "…", elapsed_ms: 6600, created_at: new Date().toISOString() };
    currentVersions().unshift(v);
    return { article: { ...a }, version: v, voice_notes: "常体で、問いで締める癖と「エクイティ」「ファイナンス」を使った。" };
  },
  // A brief written from the title (never saved). The native side takes ten
  // to twenty seconds; here one second is enough to see the waiting face.
  suggestBrief: async (articleId) => {
    if (scene === "editor-brief-suggesting") return never();
    await wait(readDelay || 1000);
    const a = currentArticles().find((x) => x.id === articleId);
    const title = a?.title.trim() || "この題";
    return `「${title}」について、数字の話に入る前に前提を揃える一本です。自分で売上や利益を見ている経営者に向けて書きます。同じ利益率でも分母の取り方で意味が変わること、そして自分の会社では何を分母にすべきかを、手元の例で示します。読み終えたあとに、自社の数字を一度だけ計算し直したくなる形にします。`;
  },
  getPolicy: async () => ({ ...currentPolicy(), topics: [...currentPolicy().topics] }),
  setPolicy: async (value) => { profilePolicies.set(selectedProfileId, { ...value, topics: [...value.topics] }); },
  // The editorial policy no longer carries what you often write about; the
  // cloud does (D-66), so the draft leaves that field empty.
  draftPolicy: async () => { await wait(1200); return { ...fullPolicy, topics: [] }; },
  getTopicCloud: async () => {
    const profileId = selectedProfileId;
    await wait(readDelay);
    // The saved cloud cannot be read: the box says so, and gathering again is the way on.
    if (scene === "topics-cloud-unread") throw { code: "unknown", detail: "database is locked" };
    return currentCloudView(profileId);
  },
  gatherTopicCloud: async () => {
    const profileId = selectedProfileId;
    // The first-run scene stays on the gathering line, so the quiet wait and
    // the way out of it can be seen.
    if (scene === "topics-cloud-first") return never();
    if (scene === "topics-cloud-failed") { await wait(readDelay || 800); throw { code: "codex_failed", detail: "topic cloud: empty response" }; }
    await wait(readDelay || 1500);
    const view = currentCloudView(profileId);
    const cloud = cloudForProfile(profileId);
    if (!cloud) return view;
    const gathered = { ...cloud, words: cloud.words.map((word) => ({ ...word })), material_count: view.material_count };
    gatheredCloudsByProfile.set(profileId, gathered);
    return { cloud: gathered, material_count: view.material_count };
  },
  suggestTopics: async (subjects, _direction) => {
    await wait(scene.startsWith("topics-picked") ? 200 : 1200);
    // Picked words steer the titles, so they are visibly about those subjects.
    const cash = subjects.some((w) => w === "資金繰り" || w === "撤退基準");
    const round = topicRoundsByProfile.get(selectedProfileId) ?? 0;
    const list = subjects.length > 0 ? (cash ? cashTitles : subjectTitles) : topicRounds[round % topicRounds.length];
    if (subjects.length === 0) topicRoundsByProfile.set(selectedProfileId, round + 1);
    const taken = new Set(currentArticles().map((a) => a.title.trim()));
    return list.filter((t) => !taken.has(t));
  },
  enqueueArticles: async (titles, voiceId, platformId, _effort, subjects, direction) => {
    // The subject and the angle stay on separate lines, so neither reads as
    // the other (requirement 12–13).
    const policy = currentPolicy();
    const brief = [
      policy.audience && `誰に向けて: ${policy.audience}`,
      policy.takeaway && `持ち帰ってほしいこと: ${policy.takeaway}`,
      policy.avoid && `触れないこと: ${policy.avoid}`,
      subjects.length > 0 && `題材: ${subjects.join("、")}`,
      direction.trim() && `今回の方向: ${direction.trim()}`,
      "長さは 1,500〜3,000 字。",
      "外部の事実は調べず、自分の経験と意見の範囲で書く。数字や出来事は断定しない。",
    ].filter(Boolean).join("\n");
    const made: T.Article[] = titles.map((title, i) => ({
      id: `q${nextId++}`, voice_id: voiceId, platform_id: platformId, title, body: "", brief,
      status: "draft" as T.ArticleStatus, queue: "waiting" as T.QueueState,
      created_at: new Date(Date.now() - i).toISOString(), updated_at: new Date(Date.now() - i).toISOString(),
    }));
    [...made].reverse().forEach((article) => currentArticles().unshift(article));
    stepQueue(selectedProfileId);
    return made.map(({ body, ...a }) => ({ ...a, excerpt: excerpt(body) }));
  },
  startQueue: async () => { for (const profile of mockProfiles) stepQueue(profile.id); },
  dequeueArticle: async (id) => {
    const rows = currentArticles();
    const timer = queueTimers.get(selectedProfileId);
    const i = rows.findIndex((article) => article.id === id);
    if (i < 0) return;
    if (rows[i].queue === "writing") {
      if (timer !== undefined) { window.clearTimeout(timer); queueTimers.delete(selectedProfileId); }
      rows[i].queue = null;
    } else {
      rows.splice(i, 1);
    }
    emitQueue({ kind: "changed" });
    stepQueue(selectedProfileId);
  },
  clearQueue: async () => {
    const rows = currentArticles();
    const timer = queueTimers.get(selectedProfileId);
    if (timer !== undefined) { window.clearTimeout(timer); queueTimers.delete(selectedProfileId); }
    for (let i = rows.length - 1; i >= 0; i--) {
      if (rows[i].queue === "waiting") rows.splice(i, 1);
      else if (rows[i].queue === "writing") rows[i].queue = null;
    }
    emitQueue({ kind: "changed" });
  },
  onQueueEvent: async (handler) => {
    queueHandlers.add(handler);
    const stoppedError = scene === "articles-queue-failed" ? { code: "network", detail: "fetch failed: ENOTFOUND" }
      : queueStoppedScenes.includes(scene) ? { code: "claude_not_logged_in", detail: "claude: not logged in" }
      : null;
    const timer = stoppedError ? window.setTimeout(() => handler({ kind: "stopped", error: stoppedError }), 150) : undefined;
    return () => { queueHandlers.delete(handler); if (timer !== undefined) window.clearTimeout(timer); };
  },
  appVersion: async () => "0.2.0",
  fetchUpdate: async () => {
    await wait(300);
    if (scene === "update-latest") return null;
    return { version: "0.3.0", current_version: "0.2.0", notes: "## Lita v0.3.0\n\n- 版履歴に差分表示\n- note の下書き保存", date: "2026-09-30T00:00:00Z" };
  },
  installUpdate: async (onEvent) => {
    onEvent({ event: "started", data: { content_length: 12_000_000 } });
    for (let i = 1; i <= (scene === "update-downloading" ? 4 : 10); i++) { await wait(150); onEvent({ event: "progress", data: { downloaded: i * 1_200_000, content_length: 12_000_000 } }); }
    if (scene === "update-downloading") return never();
    onEvent({ event: "finished", data: null });
  },
  restartApp: async () => {},
  onCheckUpdate: async () => () => {},
  // The launch scenes hold one of the two checks open so a screenshot catches
  // the launch screen: "launch" waits on the sign-in, "launch-slow" waits on
  // the engine check long enough for the slow line to appear.
  engineStatus: async () => (scene === "launch-slow" ? never<T.EnginesStatus>() : { selected: mockEngine, codex, claude }),
  selectEngine: async (engine) => { mockEngine = engine; },
  sessionStatus: async () => (scene === "launch" ? never<T.SessionStatus>() : session),
  signIn: () => (scene === "signing-in" ? never() : Promise.resolve<T.SessionStatus>({ status: "signed_in", email: "kuno@muumoo.online" })),
  cancelSignIn: async () => {},
  signOut: async () => ({ status: "signed_out" }),
  listProfiles: async () => mockProfiles.map((profile) => ({ ...profile })),
  selectedProfile: async () => {
    const profile = mockProfiles.find((item) => item.id === selectedProfileId);
    if (!profile) throw { code: "unknown", detail: "No selected mock Profile." };
    return { ...profile };
  },
  createProfile: async (name) => {
    const profile: T.PublishingProfile = { id: `profile-${nextProfileId++}`, name, is_initial: false, created_at: new Date().toISOString() };
    mockProfiles = [...mockProfiles, profile];
    profileVoices.set(profile.id, []);
    articlesByProfile.set(profile.id, []);
    versionsByProfile.set(profile.id, []);
    profilePolicies.set(profile.id, { ...emptyPolicy, topics: [] });
    topicRoundsByProfile.set(profile.id, 0);
    selectedProfileId = profile.id;
    return { ...profile };
  },
  renameProfile: async (id, name) => {
    const profile = mockProfiles.find((item) => item.id === id);
    if (!profile) throw { code: "invalid_input", detail: "No such mock Profile." };
    profile.name = name;
  },
  selectProfile: async (id) => {
    const profile = mockProfiles.find((item) => item.id === id);
    if (!profile) throw { code: "invalid_input", detail: "No such mock Profile." };
    selectedProfileId = id;
    return { ...profile };
  },
  listVoices: async () => {
    await wait(readDelay);
    // The voices cannot be listed, so there is no voice to add writing to.
    if (scene === "topics-add-unlisted") throw { code: "unknown", detail: "database is locked" };
    return currentVoices().map((item) => ({ id: item.id, name: item.name, created_at: item.created_at, updated_at: item.updated_at, source_count: item.voice_sources.length }));
  },
  getVoice: async (id) => { await wait(readDelay); const found = findCurrentVoice(id); return found ? cloneVoice(found) : null; },
  deleteVoice: async (id) => {
    const voices = currentVoices();
    if (!voices.some((item) => item.id === id)) return false;
    profileVoices.set(selectedProfileId, voices.filter((item) => item.id !== id));
    return true;
  },
  renameVoice: async (id, name) => { updateCurrentVoice(id, (value) => ({ ...value, name })); },
  updateVoiceProfile: async (id, profile) => updateCurrentVoice(id, (value) => ({ ...value, profile })),
  createVoice: () => never(),
  listVoicePresets: async () => [{ id: "pr-polite-ja", name: "広報のです・ます", one_line: PRESET_ONE_LINE }],
  createVoiceFromPreset: async (_id) => {
    const created = { ...presetVoice, id: selectedProfileId === initialProfile.id ? presetVoice.id : `${selectedProfileId}:${presetVoice.id}` };
    profileVoices.set(selectedProfileId, [cloneVoice(created)]);
    return cloneVoice(created);
  },
  addVoiceSources: async (id, added) => {
    await wait(300);
    if (scene === "topics-add-failed") throw { code: "network", detail: "fetch failed: ENOTFOUND csfvqpqzvcorqlsmfjwb.supabase.co" };
    return updateCurrentVoice(id, (value) => ({
      ...value,
      voice_sources: [...value.voice_sources, ...added.map((source, index) => ({ id: `n${index}`, kind: source.kind, origin: source.origin, account: source.account, body: source.body, created_at: new Date().toISOString() }))],
    }));
  },
  removeVoiceSource: async (id, sourceId) => updateCurrentVoice(id, (value) => ({ ...value, voice_sources: value.voice_sources.filter((source) => source.id !== sourceId) })),
  removeVoiceSources: async (id, kind, account) => updateCurrentVoice(id, (value) => ({ ...value, voice_sources: value.voice_sources.filter((source) => !(source.kind === kind && source.account === account)) })),
  rebuildVoice: () => never(),
  cancelVoiceBuild: async () => {},
  materialBudget: async () => ({ per_piece_chars: 1500, total_chars: 20000 }),
  platforms: async () => [{ id: "medium", name: "Medium", max_chars: null, rules: "" }, { id: "note", name: "note", max_chars: null, rules: "" }, { id: "x", name: "X", max_chars: 280, rules: "" }],
  previewPrompt: async (_v, brief) => `You are writing as the author described below.\n\nVoice:\n${JSON.stringify(profile, null, 2)}\n\nBrief:\n${brief}\n\nPlatform: X (280 chars)`,
  generateDraft: async (_v, _b, _p, _e, previous) => {
    if (scene === "write-generating") return never();
    if (previous) { await wait(300); return { draft: { id: "d2", brief_id: "b1", body: draftBody, prompt_sent: "", model: null, elapsed_ms: 6000, status: "pending", created_at: new Date().toISOString(), decided_at: null }, voice_notes: "前の下書きを削って収めた。" }; }
    if (scene === "write-error") { await wait(300); throw { code: "codex_quota", detail: "stream error: quota exceeded (429)" }; }
    await wait(300);
    const body = scene === "write-over" ? longBody : draftBody;
    return {
      draft: { id: "d1", brief_id: "b1", body, prompt_sent: "", model: null, elapsed_ms: 6600, status: "pending", created_at: new Date().toISOString(), decided_at: null },
      voice_notes: "常体で、問いで締める癖と「エクイティ」「ファイナンス」を使った。",
    };
  },
  cancelGenerate: async () => {},
  setDraftStatus: async () => {},
  copyText: async () => {},
  importNote: async () => {
    // Play a plausible article-by-article read over the wait; the connecting scene
    // stops partway so a screenshot catches the count mid-climb.
    const steps = scene === "intake-connecting" ? [0, 1, 2, 3, 4] : [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    const timers = steps.map((i) => setTimeout(() => emitImport({ kind: "note", done: i, total: 10 }), 60 * i + 40));
    try {
      await wait(scene === "intake-connecting" ? 60_000 : 800);
    } finally {
      timers.forEach(clearTimeout);
    }
    return { pieces, total: 12, skipped_paid: 1, recent_only: false };
  },
  importMedium: async () => { await wait(300); return { pieces, total: null, skipped_paid: 0, recent_only: true }; },
  readTalk: async (text, known) => mockReadTalk(text, known),
  importXArchive: async () => { await wait(300); return { pieces, total: null, skipped_paid: 0, recent_only: false }; },
  onImportProgress: async (handler) => { importHandlers.add(handler); return () => { importHandlers.delete(handler); }; },
  onVoiceProgress: async (handler) => {
    const stages: T.VoiceProgress[] = [{ stage: "reading", done: 0, total: 3 }, { stage: "reading", done: 2, total: 3 }, { stage: "thinking" }, { stage: "checking" }, { stage: "extracted" }];
    const timers = stages.map((p, i) => setTimeout(() => handler(p), 800 * (i + 1)));
    return () => timers.forEach(clearTimeout);
  },
  openExternal: async () => {},
};
