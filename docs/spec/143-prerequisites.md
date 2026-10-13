# Epic #143 — 人の事前作業

Epic: https://github.com/KendrickMalar/lita/issues/143（生成エンジンに Claude Code を追加し、Codex と選べるようにする）

作成: 2026-10-13（仕様策定）。基本設計で状態を確かめ直し、結果は基本設計の設計書（`docs/spec/`）に書く。

実機での受け入れ検証（実際の `claude` と `codex` を起動して生成する）に使う。どれも人にしかできない作業で、Lita もエージェントも認証情報に触れない。

| ID | 事前作業 | 確かめ方（コマンド） | 合格の条件 | 2026-10-13 の状態 | 依頼の文面 |
|---|---|---|---|---|---|
| P1 | Claude Code CLI をインストールする | `claude --version` | 終了コード 0 で版が出る | 済（2.1.296） | 「ターミナルで Claude Code をインストールし、`claude --version` で版が出ることを確かめてください。」 |
| P2 | Claude Code に自分の契約でログインする | `claude auth status --json` | `"loggedIn": true` | 済（claude.ai でログイン） | 「ターミナルで `claude auth login` を実行してログインし、`claude auth status` で loggedIn が true になることを確かめてください。」 |
| P3 | Codex CLI をインストールする | `codex --version` | 終了コード 0 で版が出る | 済（codex-cli 0.157.0） | 「ターミナルで Codex CLI をインストールし、`codex --version` で版が出ることを確かめてください。」 |
| P4 | Codex にログインする | `codex login status` | 終了コード 0 | 済（ChatGPT でログイン） | 「ターミナルで `codex login` を実行してログインし、`codex login status` が成功することを確かめてください。」 |

注意:

- 確かめ方は、状態を読むだけのコマンドに限る。`~/.claude` や `~/.codex` の認証ファイル、キーチェーンは読まない（AGENTS.md の制約 1）。
- `claude auth status --json` の出力にはメールアドレスや組織名が入る。記録や報告には `loggedIn` と `authMethod` だけを書く。
