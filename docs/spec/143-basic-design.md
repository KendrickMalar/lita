# Epic #143 — 基本設計：生成エンジンに Claude Code を足し、Codex と選べるようにする

| 項目 | 内容 |
|---|---|
| Epic | https://github.com/KendrickMalar/lita/issues/143 |
| 版 | v1（2026-10-13、基本設計工程） |
| 正本 | 仕様は Epic #143。この文書は基本設計の正本で、Feature Issue はこの版を参照する |
| 人の事前作業 | `docs/spec/143-prerequisites.md`（再確認の結果は 9 章） |

## 1. 一言サマリー

`crates/lita-codex` を `crates/lita-engine` に改名し、その中に Claude Code CLI を起動する `ClaudeCli` と、Codex／Claude Code を束ねる `Engine` と、端末に保存する選択（`engine.json`）を置く。Tauri はコマンドで選択の保存と両エンジンの状態を返し、各生成の開始時に保存された選択でエンジンを決める。画面は左下のアカウントメニューに「生成エンジン」の2択と状態を出し、止まった手順の画面からも切り替えられるようにする。

## 2. 決定事項

| 論点 | 決定 | 理由・補足 |
|---|---|---|
| B1 crate の構成 | **`crates/lita-codex` を `crates/lita-engine`（crate 名 `lita_engine`）に改名する。CodexCli は `codex.rs` に移し、`claude.rs`（ClaudeCli）・`engine.rs`（EngineKind・Engine）・`choice.rs`（選択の保存）を足す。プロンプト・スキーマ・文体（post／topics／voice／presets／sample）は同じ crate に残す** | ユーザーの選択（2026-10-13 基本設計の AskUserQuestion）。中身と名前を合わせる |
| B2 起動時の入口 | **選択中のエンジンが使えるときだけ本体に入る。止まった手順の画面に「Claude Code を使う／Codex を使う」の1行を足し、そこからも切り替えられる。新しい画面は作らない** | ユーザーの選択（同上）。Codex を入れていない人も Claude Code で始められる |
| B3 メニューの形 | **アカウントメニューの「Codex 準備完了」の1行を、「生成エンジン」の見出しと2つの選択肢（menuitemradio。名前・状態・選択中のチェック）に置き換える。使えないエンジンも選べる。選択中が使えないときだけ、その下に「直し方を見る」** | ユーザーの選択（同上）。D-80 のメニューの並び（プロフィール → 名前変更／新規 → アカウントの情報 → サインアウト）は保つ |
| B4 初期 setup | **lang-boilerplates は入れない。新しい依存も足さない。既存の lock で `npm ci` と cargo を動かし、`cargo test --workspace`・`cargo clippy --workspace --all-targets`・`npm run build` の合格を確かめる** | ユーザーの選択と一括承認（同上。8 章）。Rust の Tauri 用は単体 crate 前提（workspace は対象外）、TS 用は pnpm 前提で、lita の構成と合わない |
| B5 Claude Code の起動 | **3 章の引数。`--setting-sources ""` で利用者の CLAUDE.md・hooks・プラグインを外し、`--tools ""` でツールを外す。`--bare` は使わない** | `--bare` は OAuth とキーチェーンを読まず、claude.ai でログインした利用者が使えない（`claude --help`、2.1.296）。実測は 3.4 |
| B6 選択の保存先 | **Tauri の `app_config_dir()/engine.json`。中身は `{"version":1,"engine":"codex"|"claude"}`。読めない・壊れた・未知の値は Codex** | REQ002。順番待ちは Rust の裏で動くので、Rust 側が読める場所に置く。Supabase は使わない |
| B7 切替の効き方 | **各生成の開始時に `engine.json` を読み、そのエンジンで最後まで動かす。順番待ちは記事ごとに読み直す。文体の作成（多段）は開始時に1回だけ決める** | REQ003「次の生成から、書いている1本はそのまま」 |
| B8 エラーの符号 | **Codex の符号（`codex_*`）は今のまま。Claude Code は `claude_not_installed`・`claude_not_logged_in`・`claude_quota`・`claude_schema`・`claude_failed` を足す。`cancelled` は共通** | 直し方のコマンドがエンジンで違う（`codex login`／`claude auth login`）。Codex の失敗の扱いを変えない（制約） |
| B9 中止 | **Claude Code の子は自分のプロセスグループで起動し（`CommandExt::process_group(0)`）、中止ではグループごと止める（`/bin/kill -KILL -<pgid>`、失敗時は `child.kill()`）。新しい crate は足さない。Codex の中止は今のまま** | REQ005「中止で子プロセスを残さない」。Codex の動きは変えない（制約） |
| B10 実機の受け入れ | **lita-engine の example `engine_trial` で、実際の claude（投稿・題・文体の作成）と codex（投稿）を起動する。文章は `sample.rs` の架空の見本だけ** | AC005。範囲・回数はユーザーの承認（8 章） |

## 3. Claude Code の呼び出し方（REQ004）

### 3.1 引数

```
claude -p
  --output-format json
  --json-schema <スキーマの JSON 文字列>
  --tools ""
  --setting-sources ""
  --strict-mcp-config
  --disable-slash-commands
  --no-session-persistence
  --effort <low|medium|high>
```

- プロンプトは標準入力で渡し、書き終えたら閉じる。作業ディレクトリは `Request.working_dir`（Lita のデータディレクトリ。git ではない）。
- `--model` は付けない（既定のモデル）。`Request.model` が `Some` のときだけ `--model` を付ける（今は誰も使わない）。
- 推論量は Codex と同じ対応：`Effort::Fast`→`low`、`Quality`→`medium`、`Best`→`high`。
- 環境変数は足さない・消さない（利用者の認証は claude 自身に任せる）。
- 標準出力は1つの JSON（`type: "result"`）。値は `structured_output` から取る。

### 3.2 状態の確かめ方（preflight）

| 手順 | コマンド | 判定 |
|---|---|---|
| 1 | `claude --version` | 起動できない（NotFound）→ `NotInstalled`。出力 `2.1.296 (Claude Code)` の先頭の語を版とする |
| 2 | `claude auth status --json` | `loggedIn` が `true` → `Ready`、`false` → `NotLoggedIn`。JSON でない・`loggedIn` が無い → エラー（`Error`） |

- `claude auth status` の出力にはメールアドレス・組織名が入る。**読むのは `loggedIn` だけ**。エラーの詳細・ログ・テストの期待値に生の出力を入れない。
- 生成の直前にも同じ `auth status` で未ログインを先に見つけ、`NotLoggedIn` で止める（Codex の `logged_in()` と同じ考え）。
- Claude Code には `ConfigBroken` に当たる状態を作らない。

### 3.3 失敗の分類

| 種類 | 判定（上から順） |
|---|---|
| Cancelled | 中止の旗が立っている |
| NotLoggedIn | 結果の `result` に `Not logged in`・`/login`・`401`・`unauthorized`・`invalid api key`・`oauth token`、または `api_error_status` が 401 |
| QuotaExhausted | `api_error_status` が 429、または `result` に `usage limit`・`limit reached`・`hit your limit`・`rate limit`・`quota` |
| SchemaMismatch | `subtype` が `error_max_structured_output_retries`、`result` に `structured output`・`schema`、または成功なのに `structured_output` が無い・型に合わない |
| Unknown | それ以外（`result` と標準エラーの末尾を詳細に残す） |

- 失敗の目印は「終了コードが 0 でない」または「`is_error: true`」。
- 実測（2026-10-13、2.1.296、空の `CLAUDE_CONFIG_DIR`）：未ログインは終了コード 1、`{"is_error":true,"terminal_reason":"api_error","result":"Not logged in · Please run /login",...}`。この形はテストの見本に使ってよい（個人情報を含まない）。上限・スキーマ不一致は再現できないので、上の語で判定し、見本は架空と明記する。

### 3.4 実測の記録（2026-10-13、claude 2.1.296）

- 3.1 の引数でスキーマ付きの小さな依頼が終了コード 0、`structured_output` に値が入る。
- `--setting-sources ""` ありの入力は 3,270 トークン、なしは 13,418 トークン。利用者の CLAUDE.md・規則が外れている。

## 4. lita-engine の契約（F001 が作り、F002 が使う）

```rust
// crates/lita-engine/src/lib.rs（共通の型。Codex の今の型を引き継ぐ）
pub struct Request { prompt, schema, model: Option<String>, effort: Effort, working_dir }
pub struct Run<T> { value: T, events: Vec<Event>, elapsed: Duration }
pub enum Effort { Fast, Quality, Best }
pub enum Preflight { Ready{version}, NotLoggedIn{version}, NotInstalled, ConfigBroken{version, detail} }
pub enum FailureKind { NotLoggedIn, QuotaExhausted, SchemaMismatch, Cancelled, Unknown }
pub struct RunFailure { engine: EngineKind, kind: FailureKind, exit_code, message, stderr_tail }
pub fn assert_no_credentials(prompt: &str)

// engine.rs
#[serde(rename_all = "lowercase")] pub enum EngineKind { Codex, Claude }   // 既定は Codex
impl EngineKind { pub fn display_name(self) -> &'static str }              // "Codex" / "Claude Code"
pub enum Engine { Codex(CodexCli), Claude(ClaudeCli) }
impl Engine {
    pub fn on_path(kind: EngineKind) -> Self;
    pub fn kind(&self) -> EngineKind;
    pub fn preflight(&self) -> anyhow::Result<Preflight>;
    pub fn run_typed<T>(&self, req: &Request, on_event: impl FnMut(&Event)) -> anyhow::Result<Run<T>>;
    pub fn run_typed_cancellable<T>(&self, req, on_event, cancel: Arc<AtomicBool>) -> anyhow::Result<Run<T>>;
}

// codex.rs：今の CodexCli をそのまま移す（引数・分類・中止は変えない）
// claude.rs：ClaudeCli { on_path(), at(path), preflight(), run_typed_cancellable() }（3 章）
// choice.rs
pub fn load_choice(path: &Path) -> EngineKind;                 // 無い・壊れた・未知 → Codex
pub fn save_choice(path: &Path, kind: EngineKind) -> anyhow::Result<()>;  // 一時ファイル→rename
```

- `voice::pipeline::extract` などエンジンを受け取る関数は `&CodexCli` ではなく `&Engine` を受け取る。
- Claude Code の `Run.events` は空でよい（今、画面は事件の流れを使っていない）。
- F001 の段階では、src-tauri は `Engine::on_path(EngineKind::Codex)` で今と同じ動きのままコンパイルが通るようにだけ直す（改名に伴う import の書き換えを含む）。選択の配線は F002。

## 5. Tauri コマンドと画面の型の契約（F002 と F003 が並行で使う）

| コマンド | 引数 | 返り値 |
|---|---|---|
| `engine_status`（`codex_status` を置き換える） | なし | `EnginesStatus` |
| `select_engine` | `engine: "codex" \| "claude"` | なし（`engine.json` に保存） |

```ts
// src/platform/types.ts
export type EngineKind = "codex" | "claude";
export type EngineStatus =                     // 今の CodexStatus と同じ形
  | { status: "ready"; version: string }
  | { status: "not_logged_in"; version: string }
  | { status: "not_installed" }
  | { status: "config_broken"; version: string; message: string }   // Codex だけ
  | { status: "error"; message: string };
export type EnginesStatus = { selected: EngineKind; codex: EngineStatus; claude: EngineStatus };
// Host: codexStatus() を消し、engineStatus(): Promise<EnginesStatus> と selectEngine(engine): Promise<void> を足す
```

- `engine_status` は2つの preflight を並行に（`spawn_blocking`）走らせる。
- 生成のコマンド（投稿・記事・ブリーフ・題・編集方針・語の雲・文体の作成と学び直し・試し書き）と順番待ちは、開始時に `load_choice(app_config_dir/engine.json)` でエンジンを決める。順番待ちの停止は、記事ごとに選択中のエンジンの preflight で判断し、符号は B8（Codex は今の `codex_not_installed`・`codex_not_logged_in`・`codex_config_broken`、Claude Code は `claude_not_installed`・`claude_not_logged_in`、確かめられないときは `unknown`）。
- もう片方のエンジンへ自動で切り替える経路・勧める文言は作らない。

### UiError の符号（`src-tauri/src/errors.rs`・`src/errors.ts`）

| RunFailure | Codex | Claude Code |
|---|---|---|
| NotLoggedIn | `codex_not_logged_in` | `claude_not_logged_in` |
| QuotaExhausted | `codex_quota` | `claude_quota` |
| SchemaMismatch | `codex_schema` | `claude_schema` |
| Unknown | `codex_failed` | `claude_failed` |
| Cancelled | `cancelled` | `cancelled` |

## 6. 画面（REQ001・REQ006）

- アカウントメニュー（B3）：「生成エンジン / Engine」の見出し、`Codex` と `Claude Code` の2行。各行に状態（使える／未ログイン／未インストール／設定に問題（Codex のみ）／確認できない／確認中）。選択中にチェック。押すと `select_engine` を呼び、状態は持ったまま選択だけ替える。選択中が使えないときだけ「直し方を見る」を出す。
- 左下の警告（pill）は選択中のエンジンについてだけ出す（例：「Claude Code にログインしていません」）。
- 手順の画面（B2）：選択中のエンジンの直し方を出す。Claude Code は未ログインなら `claude auth login`、未インストールなら公式の導入ページ（https://code.claude.com/docs/en/setup 。実装時に到達を確かめる）。下に「Claude Code を使う」／「Codex を使う」の1行。
- 決め打ちの「Codex」の文言（`write.preview`・`write.generatingHint`・`privacy.note`・`status.codexOk` など）は `{engine}` で選択中の名前を入れる。Codex 固有の直し方（`codex login`・`~/.codex/config.toml`）の文言は Codex のときだけ出す。ja/en の両方。
- モック（`npm run mock`）に足すシーン（名前は実装で決めてよい。`src/platform/mock.ts` の先頭の一覧に書く）：メニューを開いた状態で (a) Codex 選択・両方使える、(b) Claude Code 選択・使える、(c) Claude Code 選択・未ログイン（直し方のリンク）、(d) Claude Code 選択・未インストール、手順の画面で (e) Claude Code 未ログイン、(f) Claude Code 未インストール、(g) 順番待ちが Claude Code の未ログインで止まった記事一覧。

## 7. Feature 分解・依存・Wave

| 案 | Feature | editScope | 依存 | Wave |
|---|---|---|---|---|
| F001 | setup 確認＋lita-engine（改名・Codex の移設・ClaudeCli・Engine・選択の保存・実機用 example） | `crates/**`、`Cargo.toml`、`Cargo.lock`、`src-tauri/Cargo.toml`、`src-tauri/src/**` | なし | 1 |
| F002 | Tauri：選択の保存・状態コマンド・生成と順番待ちの配線・エラー符号 | `src-tauri/**` | F001 | 2 |
| F003 | 画面：メニュー・手順の画面・文言（ja/en）・モックのシーン | `src/**`、`index.html` | F001 | 2 |
| F004 | 文書：AGENTS.md・README・decisions（新しい D 番号） | `AGENTS.md`、`README.md`、`docs/pm/**` | F001 | 2 |
| F005 | 通し確認（実機の受け入れ・全体の検査と、その合格に要る修正） | `.` | F002・F003・F004 | 3 |

- F001 が src-tauri を触るのは、改名と `&Engine` への置き換えでコンパイルを通すためだけ。Wave 1 は F001 単独なので競合しない。
- F003 は 5 章の型とモックで進める。実際のコマンドとのつながりは F005 で `npm run build` と実機で確かめる。
- F004 は 3 章・5 章の決定を文書に写す。`docs/pm/` の過去の記録は書き換えない。
- `.github/workflows/homebrew.yml` の案内文（Codex だけを書く）は CI/CD の変更になるので本 Epic では触らない（残論点）。

### 受け入れ要件の割り当て

| AC | Feature | 備考 |
|---|---|---|
| AC002・AC003（Epic と同じ） | F001 | 選択の保存のテスト、起動引数のテストと認証情報の grep |
| AC101〜AC104（F001 固有） | F001 | 状態判定・失敗分類・中止・Codex の不変・改名後の全体の検査・example |
| AC201〜AC203（F002 固有） | F002 | コマンド・切替の効き方・順番待ち・エラー符号 |
| AC001（Epic と同じ）・AC301〜AC302 | F003 | モックのスクリーンショット、build、文言 |
| AC006（Epic と同じ） | F004 | 文書の差分 |
| AC004・AC005・AC007（Epic と同じ） | F005 | 自動テスト一式、実機、Epic ブランチ先端の検査 |

## 8. 承認の記録（2026-10-13、基本設計工程の AskUserQuestion）

### 8.1 初期 setup の一括承認

承認した範囲：

1. 各 worktree（`~/.config/superpowers/worktrees/lita/` の下）で `npm ci`。既存の `package-lock.json` どおり。install script を持つのは `fsevents` 2.3.3 だけ。
2. workspace で `cargo build`・`cargo test`・`cargo clippy`。`Cargo.lock` 固定で crate を `~/.cargo/registry` へ取得。build.rs は既存の依存のものだけ。
3. `Cargo.lock` の変更は `lita-codex`→`lita-engine` の改名だけ。新しい依存（crate・npm）は足さない。
4. ボイラープレートは入れない。Git フック・global ツール・OS 設定は変えない（global の `core.hooksPath=~/.git-hooks` はそのまま。local の hooksPath は設定しない）。
5. 合格条件は `cargo test --workspace`・`cargo clippy --workspace --all-targets`・`npm run build` が終了コード 0。

範囲外（新しい依存、版・取得元の変更、既存設定の上書き、Git フック・OS の変更）が要るときは、その部分を止めて最終報告に書く。

### 8.2 実機の受け入れ（外部の CLI の起動）

- 担当：実装・検証工程の親と、F005 の担当（manager が tester に実行させる）。
- 手段：lita-engine の example `engine_trial`。文章は `crates/lita-engine/src/sample.rs` の架空の見本だけ。Lita のアカウント・Supabase は使わない。
- 回数：claude は投稿 1・題の提案 1・文体の作成 1（中で数回呼ぶ）、codex は投稿 1。失敗のやり直しを含め Feature ごとに最大 3 周。
- 認証情報は読まない。`claude auth status` のメール・組織名は記録しない。

外部への書き込み（画像のアップロードなど）は無い。

## 9. 人の事前作業の再確認（2026-10-13 18:13 JST）

| ID | コマンド | 結果 |
|---|---|---|
| P1 | `claude --version` | 終了コード 0、`2.1.296 (Claude Code)`。合格 |
| P2 | `claude auth status --json`（`loggedIn` と `authMethod` だけを抜き出した） | `loggedIn: true`、`authMethod: claude.ai`。合格 |
| P3 | `codex --version` | 終了コード 0、`codex-cli 0.157.0`。合格 |
| P4 | `codex login status` | 終了コード 0。合格 |

新しく分かった前提：`--bare` は claude.ai のログインを使えない（B5）。

## 10. リスクと残論点

| リスク | 手当て |
|---|---|
| Claude Code の失敗の文言が版で変わる | 3.3 の語を幅広く持ち、分類できなければ Unknown で詳細を出す。見本はテストに固定する |
| `--json-schema` が Codex 用の厳しいスキーマ（`additionalProperties: false` など）を受け付けない | F001 の example と F005 の実機で確かめる。合わなければ F005 でスキーマの渡し方を直す（プロンプトとスキーマ自体は変えない） |
| 改名の差分が大きく、他の作業中ブランチ（`feat/publishing-profiles` など）と衝突する | 改名は F001 に閉じ、早く Epic へ統合する |

残論点：

- `.github/workflows/homebrew.yml` の案内文を Claude Code にも触れる形にするか（CI/CD の変更のため本 Epic の対象外）。
- Notion の仕様ページへの反映（Epic の対象外）。
