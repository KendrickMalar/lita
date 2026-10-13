// Types shared by every host implementation (tauri.ts, mock.ts).


/** Which CLI writes for Lita. Codex is the default. */
export type EngineKind = "codex" | "claude";
export type EngineStatus =
  | { status: "ready"; version: string }
  | { status: "not_logged_in"; version: string }
  | { status: "not_installed" }
  /** Codex only: its config file cannot be read. */
  | { status: "config_broken"; version: string; message: string }
  | { status: "error"; message: string };
/** The engine in use and the last known state of both. */
export type EnginesStatus = { selected: EngineKind; codex: EngineStatus; claude: EngineStatus };

export type SessionStatus =
  | { status: "restoring" }
  | { status: "signed_out" }
  | { status: "signed_in"; email: string | null };


export type PublishingProfile = {
  id: string;
  name: string;
  is_initial: boolean;
  created_at: string;
};
export type SourceKind = "paste" | "file" | "note" | "medium" | "x";

export type SourceInput = { kind: SourceKind; origin: string | null; account: string | null; body: string };

export type VoiceProfile = {
  language: string;
  first_person: string;
  formality: string;
  tone: string[];
  sentence_endings: string[];
  avg_sentence_length_chars: number;
  preferred_words: string[];
  avoided_words: string[];
  opens_with: string;
  closes_with: string;
  uses_emoji: boolean;
  representative_excerpts: { excerpt: string; why: string }[];
  /** Codex's one sentence about the voice; empty on older profiles. */
  one_line: string;
  // Everything below arrives from the stricter extraction (2026-09-23) and is
  // absent on profiles made before it. The screen shows nothing extra when a
  // field is missing or empty.
  /** Counted by Rust, not guessed. Not shown on screen (no numbers). */
  measured?: Measured;
  /** Words this writer spells in kana (出来る→できる). */
  kana_choices?: string[];
  /** How the writing moves: repetition, turns, the shape of an argument. */
  rhetoric?: string;
  /** Whether examples are lived or general, and how numbers appear. */
  examples_and_numbers?: string;
  /** Things this writer never does, though writing of this kind often does. */
  never_does?: string[];
  /** How a word that sounds like you is actually used. */
  word_usage?: { word: string; usage: string }[];
  /** Kept for the data only: topics, not voice. Never shown, never generated from. */
  topic_words?: string[];
  /** Per field: how sure the extraction is, and the quotes behind it. */
  backing?: Record<string, Backing>;
};

export type Measured = {
  pieces: number;
  chars: number;
  sentences: number;
  sentence_length: { median: number; p10: number; p90: number; short_pct: number; long_pct: number };
  paragraphs: { count: number; median_chars: number; sentences_per_paragraph_x10: number; one_sentence_pct: number };
  punctuation: Record<string, number>;
  script: { kanji_pct: number; hiragana_pct: number; katakana_pct: number; latin_pct: number };
  endings: { form: string; count: number; pieces: number }[];
  first_person: { form: string; count: number; pieces: number }[];
  emoji: number;
};

export type Confidence = "high" | "medium" | "low";

/** `piece` indexes into `voice.voice_sources`, in the same order. */
export type Backing = {
  confidence: Confidence;
  evidence: { piece: number; quote: string }[];
  note?: string;
};

export type VoiceSource = {
  id: string;
  kind: SourceKind;
  origin: string | null;
  /** note account / Medium handle / "archive" for X; null for pasted text and files (#68). */
  account: string | null;
  body: string;
  created_at: string;
};

export type Voice = {
  id: string;
  name: string;
  profile: VoiceProfile;
  created_at: string;
  updated_at: string;
  voice_sources: VoiceSource[];
};

/** A voice bundled with Lita (D-70): picked, it becomes the person's own. */
export type VoicePreset = { id: string; name: string; one_line: string };

export type VoiceSummary = {
  id: string;
  name: string;
  created_at: string;
  updated_at: string;
  source_count: number;
};

export type ImportProgress = { kind: "note" | "medium"; done: number; total: number | null };
export type VoiceProgress =
  | { stage: "started" | "thinking" | "checking" | "extracted" | "saved" }
  | { stage: "reading"; done: number; total: number };

export type Piece = { title: string | null; url: string | null; published_at: string | null; text: string };

/// How native commands fail: a stable code plus the raw detail.
export type UiError = { code: string; detail: string };

export type MaterialBudget = { per_piece_chars: number; total_chars: number };

export type Platform = { id: string; name: string; max_chars: number | null; rules: string };
/** "best" is what every article is written with (D-63); the others remain for older callers and experiments. */
export type Effort = "fast" | "quality" | "best";
export type DraftStatus = "pending" | "approved" | "discarded";
export type Draft = {
  id: string; brief_id: string; body: string; prompt_sent: string; model: string | null;
  elapsed_ms: number; status: DraftStatus; created_at: string; decided_at: string | null;
};
export type Generated = { draft: Draft; voice_notes: string };

export type ArticleStatus = "draft" | "approved" | "archived";
/** Where a queued article stands (article-queue, 2026-09-23). `null` once written, or never queued. */
export type QueueState = "waiting" | "writing" | "failed";
export type Article = {
  id: string; voice_id: string | null; platform_id: string; title: string; body: string; brief: string;
  status: ArticleStatus; queue: QueueState | null; created_at: string; updated_at: string;
};
export type ArticleSummary = {
  id: string; voice_id: string | null; platform_id: string; title: string; excerpt: string;
  status: ArticleStatus; queue: QueueState | null; created_at: string; updated_at: string;
};
/** The editorial policy: one per app, four short free-text fields, all optional. */
export type Policy = { audience: string; takeaway: string; topics: string[]; avoid: string };
/** One word the person keeps writing about. `weight` 1–5 is folded into three sizes on screen; never shown as a number. */
export type CloudWord = { word: string; weight: number; written: boolean };
export type TopicCloud = { words: CloudWord[]; gathered_at: string; material_count: number };
/** `material_count` is the person's own writing now; more than `cloud.material_count` means "more since". */
export type CloudView = { cloud: TopicCloud | null; material_count: number };
export type QueueEvent =
  /** A queued article changed state; re-list. */
  | { kind: "changed" }
  /** The whole queue stopped on a failure that would hit every article (Codex, sign-in, network). */
  | { kind: "stopped"; error: UiError };
/** Fields to change; anything left out stays as it is. `voice_id: null` clears it. */
export type ArticlePatch = Partial<{ voice_id: string | null; platform_id: string; title: string; body: string; brief: string; status: ArticleStatus }>;
export type VersionKind = "generated" | "shortened" | "edited" | "restored" | "manual";
export type ArticleVersion = {
  id: string; article_id: string; kind: VersionKind; title: string; body: string;
  prompt_sent: string | null; elapsed_ms: number | null; created_at: string;
};
export type ArticleWritten = { article: Article; version: ArticleVersion; voice_notes: string };
export type UpdateInfo = { version: string; current_version: string; notes: string | null; date: string | null };
export type DownloadEvent =
  | { event: "started"; data: { content_length: number | null } }
  | { event: "progress"; data: { downloaded: number; content_length: number | null } }
  | { event: "finished"; data: null };
/** A pasted conversation read on this machine (D-77). `speaker: null` is text with no name above it. */
export type TalkMessage = { speaker: string | null; body: string; seen: boolean };
export type TalkReading =
  | { kind: "plain" }
  /** Times recur but no names could be told apart: pasted as before, with one sentence. */
  | { kind: "unsure" }
  | { kind: "talk"; speakers: string[]; messages: TalkMessage[] };
export type Imported = { pieces: Piece[]; total: number | null; skipped_paid: number; recent_only: boolean };


export type Unlisten = () => void;

export type Host = {
  engineStatus: () => Promise<EnginesStatus>;
  /** Saves the choice only; the states are not read again. */
  selectEngine: (engine: EngineKind) => Promise<void>;
  sessionStatus: () => Promise<SessionStatus>;
  signIn: () => Promise<SessionStatus>;
  listProfiles: () => Promise<PublishingProfile[]>;
  selectedProfile: () => Promise<PublishingProfile>;
  createProfile: (name: string) => Promise<PublishingProfile>;
  renameProfile: (id: string, name: string) => Promise<void>;
  selectProfile: (id: string) => Promise<PublishingProfile>;
  cancelSignIn: () => Promise<void>;
  signOut: () => Promise<SessionStatus>;
  listVoices: () => Promise<VoiceSummary[]>;
  getVoice: (id: string) => Promise<Voice | null>;
  deleteVoice: (id: string) => Promise<boolean>;
  renameVoice: (id: string, name: string) => Promise<void>;
  updateVoiceProfile: (id: string, profile: VoiceProfile) => Promise<Voice | null>;
  createVoice: (name: string, sources: SourceInput[]) => Promise<Voice>;
  /** Bundled voices to start from. Static. */
  listVoicePresets: () => Promise<VoicePreset[]>;
  /** Copies a bundled voice into the person's own (no material, no evidence). */
  createVoiceFromPreset: (id: string) => Promise<Voice>;
  addVoiceSources: (voiceId: string, sources: SourceInput[]) => Promise<Voice | null>;
  removeVoiceSources: (voiceId: string, kind: SourceKind, account: string | null) => Promise<Voice | null>;
  /** Drops one piece by id (manual intake). */
  removeVoiceSource: (voiceId: string, sourceId: string) => Promise<Voice | null>;
  rebuildVoice: (voiceId: string) => Promise<Voice>;
  cancelVoiceBuild: () => Promise<void>;
  materialBudget: () => Promise<MaterialBudget>;
  platforms: () => Promise<Platform[]>;
  previewPrompt: (voiceId: string, brief: string, platformId: string, previous?: string) => Promise<string>;
  generateDraft: (voiceId: string, brief: string, platformId: string, effort: Effort, previous?: string) => Promise<Generated>;
  cancelGenerate: () => Promise<void>;
  setDraftStatus: (id: string, status: DraftStatus) => Promise<void>;
  listArticles: (status?: ArticleStatus) => Promise<ArticleSummary[]>;
  getArticle: (id: string) => Promise<Article | null>;
  updateArticle: (id: string, patch: ArticlePatch) => Promise<void>;
  deleteArticle: (id: string) => Promise<boolean>;
  listVersions: (articleId: string) => Promise<ArticleVersion[]>;
  snapshotArticle: (articleId: string, manual: boolean) => Promise<ArticleVersion | null>;
  restoreVersion: (versionId: string) => Promise<Article>;
  generateIntoArticle: (articleId: string, effort: Effort, previous?: string) => Promise<ArticleWritten>;
  // ----- article queue (requirements 2026-09-23-article-queue) -----
  getPolicy: () => Promise<Policy>;
  setPolicy: (policy: Policy) => Promise<void>;
  /** Codex drafts the four fields from the voices' sources and past articles. Does not save. */
  draftPolicy: () => Promise<Policy>;
  /** A brief drafted from the article's title (needs a title). Not saved. Stopped by `cancelGenerate`. */
  suggestBrief: (articleId: string) => Promise<string>;
  /** Ten titles, none already used. `subjects` are picked cloud words (0–3); `direction` may be empty. */
  suggestTopics: (subjects: string[], direction: string) => Promise<string[]>;
  /** One draft per title, brief filled, queued in order. Starts the queue. */
  enqueueArticles: (titles: string[], voiceId: string, platformId: string, effort: Effort, subjects: string[], direction: string) => Promise<ArticleSummary[]>;
  /** The cloud as last gathered (null until gathered) and the material count now. */
  getTopicCloud: () => Promise<CloudView>;
  /** Codex gathers the words (10–20 s) and saves them. Stopped by `cancelGenerate`. */
  gatherTopicCloud: () => Promise<CloudView>;
  /** Resumes a queue left over from last time. Safe to call when nothing is waiting. */
  startQueue: () => Promise<void>;
  /** Waiting: the article is deleted. Writing: stopped, kept as an empty draft. */
  dequeueArticle: (id: string) => Promise<void>;
  /** Everything waiting is deleted and the current one stopped. */
  clearQueue: () => Promise<void>;
  onQueueEvent: (handler: (e: QueueEvent) => void) => Promise<Unlisten>;
  appVersion: () => Promise<string>;
  fetchUpdate: () => Promise<UpdateInfo | null>;
  installUpdate: (onEvent: (e: DownloadEvent) => void) => Promise<void>;
  restartApp: () => Promise<void>;
  onCheckUpdate: (handler: () => void) => Promise<Unlisten>;
  copyText: (text: string) => Promise<void>;
  importNote: (account: string) => Promise<Imported>;
  importMedium: (handle: string) => Promise<Imported>;
  importXArchive: (contents: string, handle: string | null, includeReplies: boolean) => Promise<Imported>;
  /** Who said what in a pasted conversation. `known`: bodies added from conversations before. Local only. */
  readTalk: (text: string, known: string[]) => Promise<TalkReading>;
  onVoiceProgress: (handler: (p: VoiceProgress) => void) => Promise<Unlisten>;
  /** Real progress of a service import: done / total (total unknown until the listing arrived). */
  onImportProgress: (handler: (p: ImportProgress) => void) => Promise<Unlisten>;
  openExternal: (url: string) => Promise<void>;
};
