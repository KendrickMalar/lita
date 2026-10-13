// Turns whatever a native command threw into something a person can act
// on: a known code (see src-tauri/src/errors.rs) with the raw detail kept
// for a disclosure, never as the headline (R-5).

import type { UiError } from "./platform/host";
import { t } from "./i18n";
import type { MessageKey } from "./i18n/en";

const KNOWN = new Set([
  "not_signed_in", "session_expired", "cancelled", "sign_in_timeout", "network", "account_not_found",
  "invalid_input", "codex_not_logged_in", "codex_quota", "codex_schema", "codex_failed",
  "claude_not_installed", "claude_not_logged_in", "claude_quota", "claude_schema", "claude_failed", "unknown",
]);

export function asUiError(e: unknown): UiError {
  if (e && typeof e === "object" && "code" in e && typeof (e as UiError).code === "string") {
    const u = e as UiError;
    return { code: KNOWN.has(u.code) ? u.code : "unknown", detail: u.detail ?? "" };
  }
  return { code: "unknown", detail: String(e) };
}

export function describe(err: UiError): string {
  return t(`error.${err.code}` as MessageKey);
}
