import { useId, useRef, useState } from "react";
import type { EngineKind, EnginesStatus, UiError } from "../platform/host";
import { t } from "../i18n";
import { ENGINE_KINDS, ENGINE_NAMES, engineStateWord } from "../engine";
import { ErrorNote } from "./ErrorNote";

type Props = {
  engines: EnginesStatus | null;
  checking: boolean;
  /** Saves the choice. Resolves to the error when it could not be saved. */
  onSelect: (engine: EngineKind) => Promise<UiError | null>;
  /** Shows the steps for the engine in use ("直し方を見る"). */
  onShowSteps: () => void;
  /** Arrow-key focus: `slot` counts the menu's items from the top, and the
   *  menu that hosts this one numbers them. The engine rows come first. */
  slotBase?: number;
  itemRef?: (slot: number) => (element: HTMLButtonElement | null) => void;
  onItemKeyDown?: (event: React.KeyboardEvent<HTMLButtonElement>, slot: number) => void;
};

/** True when the engine in use cannot write yet, so the way to fix it is offered. */
export function needsSteps(engines: EnginesStatus | null): boolean {
  return engines !== null && engines[engines.selected].status !== "ready";
}

// The account menu's "生成エンジン" block: a heading, one row per engine
// (name, state, a check on the one in use), and, only while the one in use
// cannot write, a way to see how to fix it. Pressing an engine keeps the menu
// open: the check moves, and the way to fix it appears or goes in place.
export function EngineMenu({ engines, checking, onSelect, onShowSteps, slotBase = 0, itemRef, onItemKeyDown }: Props) {
  const headingId = useId();
  const [error, setError] = useState<UiError | null>(null);
  // Not `disabled`: a disabled button would drop the keyboard focus.
  const busy = useRef(false);

  const choose = async (engine: EngineKind) => {
    if (busy.current) return;
    busy.current = true;
    setError(null);
    try {
      setError(await onSelect(engine));
    } finally {
      busy.current = false;
    }
  };
  const ref = (slot: number) => itemRef?.(slot);
  const keys = (slot: number) => (event: React.KeyboardEvent<HTMLButtonElement>) => onItemKeyDown?.(event, slot);

  return (
    <div role="group" aria-labelledby={headingId}>
      <div className="menu-meta" id={headingId}>{t("engine.heading")}</div>
      {ENGINE_KINDS.map((kind, index) => {
        const chosen = engines?.selected === kind;
        return (
          <button
            key={kind}
            ref={ref(slotBase + index)}
            type="button"
            role="menuitemradio"
            aria-checked={chosen}
            className={chosen ? "profile-menu-item selected" : "profile-menu-item"}
            onKeyDown={keys(slotBase + index)}
            onClick={() => void choose(kind)}
          >
            <span className="profile-menu-name">{ENGINE_NAMES[kind]}</span>
            <span className="engine-side">
              <span className="engine-state">{engineStateWord(engines?.[kind] ?? null, checking)}</span>
              <span className="engine-check" aria-hidden="true">{chosen ? "✓" : ""}</span>
            </span>
          </button>
        );
      })}
      {error && <div className="profile-error"><ErrorNote error={error} /></div>}
      {needsSteps(engines) && (
        <button
          ref={ref(slotBase + ENGINE_KINDS.length)}
          type="button"
          role="menuitem"
          className="profile-menu-item profile-action"
          onKeyDown={keys(slotBase + ENGINE_KINDS.length)}
          onClick={onShowSteps}
        >
          {t("engine.seeSteps")}
        </button>
      )}
    </div>
  );
}
