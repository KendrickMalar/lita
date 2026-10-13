# Lita

**Lita writes your posts, in your voice — using the Codex or Claude Code login you already have.**

Lita is a desktop app for people who publish. It is made of three things: your
**Voices** (how you write, learned from your own writing), your **articles**,
and an **editor** where Lita writes from a one-line brief and you fix what you
want. Drafts sound like you, not like a chatbot.

It does not ask you for an API key. It drives the [OpenAI Codex
CLI](https://developers.openai.com/codex/cli) or Claude Code that you already
signed into (you choose one), so your login stays where it is and Lita never
touches your credentials.

> **Status: v0.2, in daily use by the author.** Articles, an editor with
> autosave and version history, long-form for note and Medium, and one or
> more Voices. See [Install](#install) and [Roadmap](#roadmap).

Site and privacy policy: <https://lita.muumoo.online/>

## Why

Most AI writing tools give you a blank box and a generic model. You end up
rewriting everything because it does not sound like you.

Lita inverts that. It learns a **Voice** from writing you have already done, and
treats that Voice as the contract every draft has to satisfy. The human job is
the brief and the approval, not the typing.

You do not need much: we measured it, and a Voice is essentially stable from
four pieces of writing. Twenty pieces you actually like beat five hundred
scraped ones, because nothing you dislike gets in.

## How it works

```
Your public writing ──▶ Voice(s) ──┐
(note, Medium, X archive,          │
 or paste / a text file)           ├──▶ Codex CLI or Claude Code ──▶ article text ──▶ you edit ──▶ copy
                                   │        (every run and edit kept as a version)
Brief, in the editor ──────────────┘
("what, for whom, what to take away")
```

1. **Point it at your writing.** Enter your note or Medium account and it pulls
   your public articles; drop in your X archive (`tweets.js`); or paste text.
   Pick the pieces you are happy with.
2. **Build a Voice.** The engine you chose distils formality, tone, sentence
   endings, the words that are yours, and how you open and close, into a
   profile you can read and edit line by line. It also writes one sentence
   about your writing for the top of the screen.
3. **Open an article.** The home screen lists what you have written. A new
   article opens the editor: your text on the left, Lita on the right.
4. **Write a brief and let Lita write.** A sentence or two: what to say, to
   whom, what they should take away. Lita runs `codex exec` or `claude -p` with
   a JSON schema and writes into the article: a post for X, or a titled,
   paragraphed piece for note or Medium. You see the exact prompt first, choose
   fast or careful, and can stop a run.
5. **Edit and keep.** Everything you type is saved as you go. Each generation,
   rewrite and restore is a version you can go back to. Copy when it is right.
6. **Tune the Voice.** Fix Lita's one-line reading of your writing, or any
   line of the details, in place. Keep a Voice per way of writing; a new
   article starts with the one you used last.

## Design principles

- **Borrowed auth.** Lita spawns the Codex CLI or Claude Code as a subprocess.
  It never reads, stores, or refreshes your OpenAI or Anthropic tokens.
- **Honest about the network.** Before writing, the app shows you exactly what
  will be sent to the engine you chose: the structured Voice and your brief,
  never your original excerpts.
- **The Voice is a document, not a black box.** Every field is visible and
  editable in the app.
- **Your data follows your login.** Voices, briefs and drafts are stored under
  your Lita account (Google sign-in, Supabase in Tokyo, row-level security) so
  they are there on another machine. No analytics, no crash reports. What is
  stored and where is spelled out in the
  [privacy policy](https://lita.muumoo.online/privacy.html). Self-hosting is
  one constant away: the Supabase URL and anon key live in
  `src-tauri/src/config.rs`.

## Requirements

- macOS (Windows and Linux are untested)
- One of these, installed and logged in:
  - [Codex CLI](https://developers.openai.com/codex/cli) (`codex login`)
  - Claude Code (`claude auth login`;
    [install](https://code.claude.com/docs/en/setup))

  You pick which one Lita uses in the account menu at the bottom left. Until
  you pick, Lita uses Codex. Claude Code is never used unless you choose it.
- A Google account, for signing in to Lita

## Installing

Apple Silicon Macs, macOS 12 or later. Download the DMG from the
[latest release](https://github.com/KendrickMalar/lita/releases/latest).

Builds are not yet signed with an Apple Developer ID, so macOS will refuse to
open the app the first time. Either allow it under System Settings › Privacy &
Security › "Open Anyway", or run:

```
xattr -d com.apple.quarantine /Applications/Lita.app
```

Once installed, Lita checks for updates when it starts and from
**Lita › アップデートを確認…**. A Homebrew cask (`KendrickMalar/lita/lita`) will
follow once builds are signed.

## Running from source

You need Rust (stable), Node.js 20+, and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```
npm install
npm run tauri dev     # opens the app against the Vite dev server
npm run tauri build   # native bundle under src-tauri/target/release/bundle
npm run mock          # every screen in a plain browser: http://localhost:1430/?scene=voice
cargo test            # Rust unit tests
```

The repository is a Cargo workspace:

| Path | What |
| --- | --- |
| `crates/lita-engine` | drives the Codex CLI and Claude Code; owns the Voice profile and the prompts |
| `crates/lita-sources` | note, Medium and X archive import |
| `crates/lita-auth` | Google sign-in through Supabase, session in the OS keychain |
| `crates/lita-store` | data access through PostgREST with the user's token |
| `src-tauri` | the Tauri shell: commands only, logic lives in the crates |
| `src/` | React + TypeScript frontend; `src/platform/` is the only place that talks to Tauri |
| `supabase/` | schema migrations and auth config |
| `site/` | the homepage and privacy policy, published to GitHub Pages |

## Roadmap

| Version | Scope |
| --- | --- |
| **v0.1** (done) | Voice from note / Medium / X archive / paste · one Voice · brief → X post → approve → copy · fast/careful · prompt preview · shorter rewrite |
| **v0.2** (done) | Articles as the home · editor with autosave and version history · long-form for note and Medium · several Voices |
| v0.3 | Distribution: signed and notarised builds on GitHub Releases, in-app updates, Homebrew cask · publisher plugins (draft to platform) · other model backends |

Slack as a Voice source was investigated and shelved: Marketplace listing
requires ten active workspaces before review, which a personal OSS project
cannot meet.

## Specification

The working specification lives in Notion and is the source of truth. This
repository carries a summary and the day-to-day project state:

- [`docs/`](docs/) — how the spec is organised and where to find it
- [`docs/pm/`](docs/pm/) — decisions, backlog, state, session handoff, and the
  measurement notes behind the design

## Contributing

Lita is MIT licensed and open from day one. It is early: the most useful
contribution right now is an opinion. Open an issue if the design above is
wrong for how you publish.

## License

[MIT](LICENSE)
