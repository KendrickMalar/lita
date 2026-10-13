import { useEffect, useState } from "react";
import { host, type VoiceProgress } from "../platform/host";
import { t } from "../i18n";
import { useEngineName } from "../engine";

// The stages are real since 2026-09-23 (voice quality): the material is
// measured, the voice is read out of it, then checked against the material
// for counter-examples with verified quotes.
const STAGES: VoiceProgress["stage"][] = ["started", "thinking", "checking", "saved"];

export function BuildingVoice({ onCancel, relearn }: { onCancel: () => void; relearn?: boolean }) {
  const engineName = useEngineName();
  const [progress, setProgress] = useState<VoiceProgress>({ stage: "started" });

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void host.onVoiceProgress((p) => setProgress(p)).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);

  const reached = progress.stage === "extracted" ? "checking" : progress.stage;
  const current = STAGES.indexOf(reached);
  const label = (s: VoiceProgress["stage"]) =>
    s === "reading" && progress.stage === "reading" && progress.total > 0
      ? t("voice.stage.readingN", { done: String(progress.done), total: String(progress.total) })
      : t(`voice.stage.${s}` as const);
  return (
    <div className="building" aria-live="polite">
      <div className="row between">
        <h2>{t(relearn ? "voice.rebuilding.title" : "voice.building.title")}</h2>
        <button className="btn" onClick={onCancel}>{t("voice.building.cancel")}</button>
      </div>
      <ol className="stages">
        {STAGES.map((s, i) => (
          <li key={s} className={i < current ? "done" : i === current ? "active" : ""} aria-current={i === current ? "step" : undefined}>
            {label(s)}
          </li>
        ))}
      </ol>
      <p className="muted small">{t("voice.building.hint")}</p>
      <footer className="privacy">{t("privacy.note", { engine: engineName })}</footer>
    </div>
  );
}
