import { t } from "../i18n";
import { Delayed } from "./Delayed";

/** How long a launch may take before the wait is worth mentioning (#111). */
const SLOW_LAUNCH_MS = 3000;

// The first second or two of the app. Nothing has been decided yet, so there
// is nothing to act on: the screen is the name on paper, the one line the
// sign-in screen also carries, and one sentence saying what is being waited
// on. The only movement is the mint dot of the wordmark, and it stops under
// `prefers-reduced-motion`. A wait past three seconds adds one quiet line.
export function Launch({ step }: { step: "session" | "engine" }) {
  return (
    <div className="launch">
      <div className="launch-stage">
        <p className="launch-mark">
          Lita<span className="dot" aria-hidden="true">.</span>
        </p>
        <p className="launch-tag">{t("app.tagline")}</p>
        <p className="launch-line" role="status">{t(step === "session" ? "session.checking" : "engine.checkingAny")}</p>
        {/* The slot keeps its height from the first paint, so nothing on
            screen moves when the slow line arrives. */}
        <div className="launch-slot">
          <Delayed ms={SLOW_LAUNCH_MS}>
            <p className="launch-slow">{t("launch.slow")}</p>
          </Delayed>
        </div>
      </div>
    </div>
  );
}
