// Which engine writes for Lita (Codex or Claude Code), and how it is named.
// The names live here and nowhere else: screens that say "the engine" take
// the name from `useEngineName()` or `ENGINE_NAMES`, so a sentence that
// applies to both never hard-codes one of them.

import { createContext, useContext } from "react";
import { t } from "./i18n";
import type { MessageKey } from "./i18n/en";
import type { EngineKind, EngineStatus } from "./platform/types";

export const ENGINE_NAMES: Record<EngineKind, string> = { codex: "Codex", claude: "Claude Code" };
/** Codex first, as the menu lists them. */
export const ENGINE_KINDS: EngineKind[] = ["codex", "claude"];
/** Claude Code's official install page (reachable on 2026-10-13). */
export const CLAUDE_INSTALL_URL = "https://code.claude.com/docs/en/setup";

/** The engine in use. App provides it; before it is known, Codex. */
export const EngineContext = createContext<EngineKind>("codex");
export function useEngine(): EngineKind {
  return useContext(EngineContext);
}
export function useEngineName(): string {
  return ENGINE_NAMES[useContext(EngineContext)];
}

/** The short word beside an engine's name in the menu. */
export function engineStateWord(status: EngineStatus | null, checking: boolean): string {
  if (checking || status === null) return t("engine.state.checking");
  const key: MessageKey =
    status.status === "ready" ? "engine.state.ready"
    : status.status === "not_logged_in" ? "engine.state.notLoggedIn"
    : status.status === "not_installed" ? "engine.state.notInstalled"
    : status.status === "config_broken" ? "engine.state.configBroken"
    : "engine.state.error";
  return t(key);
}

/** The warning for the engine in use; null when there is nothing to warn about. */
export function enginePill(engine: EngineKind, status: EngineStatus | null, checking: boolean): string | null {
  const vars = { engine: ENGINE_NAMES[engine] };
  if (checking) return t("engine.pill.checking", vars);
  switch (status?.status) {
    case "not_logged_in": return t("engine.pill.notLoggedIn", vars);
    case "not_installed": return t("engine.pill.notInstalled", vars);
    // Only Codex has a configuration file to fix; for Claude Code it reads as
    // "could not be checked", as in its steps.
    case "config_broken": return engine === "codex" ? t("codex.pill.configBroken") : t("engine.pill.error", vars);
    case "error": return t("engine.pill.error", vars);
    default: return null;
  }
}
