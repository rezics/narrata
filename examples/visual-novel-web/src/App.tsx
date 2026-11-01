import { useEffect, useState, useSyncExternalStore } from "react";
import type { MouseEvent } from "react";
import { openDemoHost, type DemoHost } from "./player-host.js";
import { PlayerStage } from "./PlayerStage.js";
import { readVnText, type VnPlayerSnapshot } from "@rezics/narrata/vn/player";

export function App() {
  const [host, setHost] = useState<DemoHost>();
  const [openError, setOpenError] = useState(false);

  useEffect(() => {
    let active = true;
    let session: DemoHost | undefined;
    void openDemoHost().then(async value => {
      session = value;
      if (!active) {
        await value.close();
        return;
      }
      setHost(value);
      void value.controller.start();
    }).catch(() => {
      if (active) setOpenError(true);
    });
    const pageHide = () => { void session?.close(); };
    window.addEventListener("pagehide", pageHide, { once: true });
    return () => {
      active = false;
      window.removeEventListener("pagehide", pageHide);
      void session?.close();
    };
  }, []);

  if (openError) {
    return <main className="boot-state"><p>The story could not be opened.</p><button type="button" onClick={() => location.reload()}>Retry</button></main>;
  }
  if (!host) return <main className="boot-state"><span className="loader" aria-hidden="true" /><p>Opening story…</p></main>;
  return <PlayerSurface host={host} />;
}

function PlayerSurface({ host }: { host: DemoHost }) {
  const snapshot = useSyncExternalStore(host.controller.subscribe, host.controller.getSnapshot, host.controller.getSnapshot);
  const [historyOpen, setHistoryOpen] = useState(false);
  const screen = snapshot.screen;
  const view = screen?.view;
  const ready = snapshot.status === "ready";
  const interaction = view?.view.interaction;
  const hasHistory = Boolean(view && view.view.history.some(item => !item.current && item.depth < view.view.depth));
  const audioUnavailable = Boolean(screen?.audioIssues.length) || snapshot.soundStatus === "unavailable";

  useEffect(() => {
    if (!import.meta.env.DEV) return;
    window.__vnTestHooks = {
      metrics: host.metrics,
      controls: host.controls,
      reader: host.reader,
      controller: host.controller,
      async switchContext(scope) { await host.switchContext(scope); },
      async save() { return host.reader.exportSave(); },
      async currentView() { return host.reader.inspect(); },
      async close() { await host.close(); },
    };
    return () => { delete window.__vnTestHooks; };
  }, [host]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!ready || historyOpen || interaction?.kind !== "advance" || event.repeat || (event.key !== "Enter" && event.code !== "Space")) return;
      const target = event.target;
      if (target instanceof HTMLElement && target.closest("button, a, input, textarea, select, [role=dialog]")) return;
      event.preventDefault();
      void host.controller.advance().catch(() => undefined);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [host, historyOpen, interaction?.kind, ready]);

  useEffect(() => {
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const onResize = () => {
      if (timeout !== undefined) clearTimeout(timeout);
      timeout = setTimeout(() => {
        timeout = undefined;
        void host.controller.refreshPresentation().catch(() => undefined);
      }, 120);
    };
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      if (timeout !== undefined) clearTimeout(timeout);
    };
  }, [host]);

  const activateStage = (event: MouseEvent<HTMLDivElement>) => {
    if (!ready || historyOpen || interaction?.kind !== "advance") return;
    const target = event.target;
    if (target instanceof HTMLElement && target.closest("button, a, [role=dialog]")) return;
    void host.controller.advance().catch(() => undefined);
  };

  const activateSound = () => {
    if (!screen || !ready) return;
    if (audioUnavailable) {
      const unlock = host.unlockAudio();
      void unlock.then(async () => {
        host.metrics.retries.audio++;
        await host.controller.retryAudio();
        if (host.controller.getSnapshot().soundStatus !== "on") {
          await host.controller.setSoundEnabled(true);
        }
      }).catch(() => undefined);
      return;
    }
    if (snapshot.soundStatus === "on") {
      void host.controller.setSoundEnabled(false).catch(() => undefined);
      return;
    }
    const unlock = host.unlockAudio();
    void unlock.then(() => host.controller.setSoundEnabled(true)).catch(() => undefined);
  };

  const stageBusy = snapshot.status === "preparing" || snapshot.status === "unavailable" || snapshot.status === "logical-error"
    || snapshot.status === "save-unknown" || snapshot.status === "reload-required";
  const blocking = snapshot.status !== "ready";

  return (
    <main className="player-shell" onClick={activateStage}>
      <PlayerStage screen={screen} />
      <header className="player-toolbar" aria-label="Story controls" onClick={event => event.stopPropagation()}>
        <button className="toolbar-control" type="button" disabled={!ready || !hasHistory} onClick={() => { void host.controller.back().catch(() => undefined); }}>
          <ArrowLeftIcon /><span>Back</span>
        </button>
        <span className="toolbar-divider" aria-hidden="true" />
        <button className="toolbar-control" type="button" disabled={!ready || !view?.view.history.length} onClick={() => setHistoryOpen(true)}>
          <HistoryIcon /><span>History</span>
        </button>
        <span className="toolbar-divider" aria-hidden="true" />
        <div className="save-status" role="status" aria-live="polite" data-testid="save-status">
          <SaveIcon /><span>{saveLabel(snapshot)}</span>
        </div>
        <button
          className={`sound-control${snapshot.soundStatus === "on" ? " is-on" : ""}`}
          type="button"
          aria-label={audioUnavailable ? "Retry sound" : snapshot.soundStatus === "on" ? "Turn sound off" : "Turn sound on"}
          aria-pressed={snapshot.soundStatus === "on"}
          disabled={!ready}
          onClick={event => { event.stopPropagation(); activateSound(); }}
        >
          <SpeakerIcon muted={snapshot.soundStatus !== "on"} />
          <span>{audioUnavailable ? "Retry sound" : snapshot.soundStatus === "on" ? "Sound on" : "Sound off"}</span>
          <span className="switch" aria-hidden="true"><span /></span>
        </button>
      </header>

      {stageBusy && (
        <div className={`state-layer${snapshot.status === "preparing" ? " is-loading" : " is-failure"}`} role={snapshot.status === "preparing" ? "status" : "alert"} aria-live="polite">
          {snapshot.status === "preparing" ? (
            <div className="loading-card"><span className="loader" aria-hidden="true" /><span>{screen ? "Loading next scene…" : "Loading scene…"}</span></div>
          ) : (
            <FailureCard snapshot={snapshot} hasScreen={Boolean(screen)} onRetry={() => {
              if (snapshot.status === "unavailable" && snapshot.target) {
                host.metrics.retries.resource++;
                void host.controller.retryResources().catch(() => undefined);
              } else if (snapshot.status === "logical-error") {
                host.metrics.retries.logical++;
                void host.controller.retryInput().catch(() => undefined);
              } else if (snapshot.status === "save-unknown") {
                host.metrics.retries.save++;
                void host.controller.retrySave().catch(() => undefined);
              } else if (snapshot.status === "reload-required") void host.controller.reloadProgress().catch(() => undefined);
              else location.reload();
            }} onReload={() => { void host.controller.reloadProgress().catch(() => undefined); }} />
          )}
        </div>
      )}

      {screen && interaction?.kind === "advance" && (
        <section className="dialogue-panel" aria-label="Current dialogue" aria-busy={snapshot.saveStatus === "saving"}>
          {readVnText(screen, interaction.speaker) && <p className="speaker-name">{readVnText(screen, interaction.speaker)}</p>}
          <p className="dialogue-copy">{readVnText(screen, interaction.line)}</p>
          <button className="continue-button" type="button" aria-label="Continue dialogue" disabled={!ready} onClick={event => {
            event.stopPropagation();
            void host.controller.advance().catch(() => undefined);
          }}>
            <span>Continue</span><ChevronIcon />
          </button>
          {audioUnavailable && <p className="audio-status" role="status">Sound is unavailable. You can continue without it.</p>}
        </section>
      )}

      {screen && interaction?.kind === "choose" && (
        <section className="choice-panel" aria-label="Story choices">
          {screen.view.view.presentation.map((item, index) => {
            const text = readVnText(screen, item.content, item.args);
            return text ? <p className="choice-prompt" key={`${item.commit}-${item.occurrence}-${index}`}>{text}</p> : null;
          })}
          <div className="choice-options">
            {interaction.options.map(option => {
              const label = readVnText(screen, option.label, interaction.args);
              return label ? (
                <button
                  className="choice-button"
                  type="button"
                  key={option.id}
                  disabled={!option.enabled || !ready}
                  onClick={event => { event.stopPropagation(); void host.controller.choose(option.id).catch(() => undefined); }}
                >{label}</button>
              ) : null;
            })}
          </div>
          {audioUnavailable && <p className="audio-status" role="status">Sound is unavailable. You can continue without it.</p>}
        </section>
      )}

      {screen && interaction?.kind === "finished" && (
        <section className="dialogue-panel ending-panel" aria-label="Story ending">
          {readVnText(screen, interaction.title) && <p className="speaker-name">{readVnText(screen, interaction.title)}</p>}
          {readVnText(screen, interaction.body) && <p className="dialogue-copy">{readVnText(screen, interaction.body)}</p>}
          {audioUnavailable && <p className="audio-status" role="status">Sound is unavailable. You can continue without it.</p>}
        </section>
      )}

      {historyOpen && ready && screen && (
        <HistoryDialog
          snapshot={snapshot}
          onClose={() => setHistoryOpen(false)}
          onSelect={commit => {
            setHistoryOpen(false);
            void host.controller.checkout(commit).catch(() => undefined);
          }}
        />
      )}

      {blocking && <span className="screen-reader-only">{snapshot.status === "submitting" ? "Saving progress" : "Story controls are unavailable"}</span>}
    </main>
  );
}

function FailureCard({ snapshot, hasScreen, onRetry, onReload }: {
  snapshot: VnPlayerSnapshot;
  hasScreen: boolean;
  onRetry: () => void;
  onReload: () => void;
}) {
  if (snapshot.status === "save-unknown") {
    return <div className="failure-card"><h1>Save status unknown</h1><p>Your last action may still be saved.</p><div className="failure-actions"><button type="button" onClick={onRetry}>Retry save</button><button type="button" className="secondary-action" onClick={onReload}>Reload saved progress</button></div></div>;
  }
  if (snapshot.status === "reload-required") {
    return <div className="failure-card"><h1>Progress needs to be checked</h1><p>The saved position could not be confirmed.</p><button type="button" onClick={onRetry}>Reload saved progress</button></div>;
  }
  if (snapshot.status === "logical-error") {
    return <div className="failure-card"><h1>Story section unavailable</h1><p>The next story section could not load.</p><button type="button" onClick={onRetry}>Retry</button></div>;
  }
  return <div className="failure-card"><h1>Scene unavailable</h1><p>{hasScreen ? "The next scene could not load." : "The scene could not load."}</p><button type="button" onClick={onRetry}>Retry</button></div>;
}

function HistoryDialog({ snapshot, onClose, onSelect }: {
  snapshot: VnPlayerSnapshot;
  onClose: () => void;
  onSelect: (commit: string) => void;
}) {
  const view = snapshot.screen?.view.view;
  const moments = [...(view?.history ?? [])]
    .filter(item => item.id !== view?.cursor && item.depth < (view?.depth ?? 0))
    .sort((left, right) => left.depth - right.depth);
  return (
    <div className="history-scrim">
      <section className="history-dialog" role="dialog" aria-modal="true" aria-labelledby="history-title" onClick={event => event.stopPropagation()}>
        <div className="history-heading"><h2 id="history-title">History</h2><button type="button" aria-label="Close history" onClick={onClose}><CloseIcon /></button></div>
        {moments.length ? (
          <ol className="history-list">
            {moments.map(item => <li key={item.id}><button type="button" onClick={() => onSelect(item.id)}>Moment {item.depth + 1}</button></li>)}
          </ol>
        ) : <p className="history-empty">No earlier moments yet.</p>}
      </section>
    </div>
  );
}

function saveLabel(snapshot: VnPlayerSnapshot): string {
  if (snapshot.saveStatus === "saving") return "Saving…";
  if (snapshot.saveStatus === "unknown") return "Save status unknown";
  return "Saved";
}

function ArrowLeftIcon() { return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m14.5 5-7 7 7 7" /></svg>; }
function HistoryIcon() { return <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3.5 2" /></svg>; }
function SaveIcon() { return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 3h12l4 4v14H3V3h2Z" /><path d="M7 3v6h10V3M7 21v-8h10v8" /></svg>; }
function SpeakerIcon({ muted }: { muted: boolean }) { return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 10v4h4l5 4V6l-5 4H4Z" />{muted ? <path d="m17 9 4 6m0-6-4 6" /> : <><path d="M16 9a4 4 0 0 1 0 6" /><path d="M18.5 6.5a8 8 0 0 1 0 11" /></>}</svg>; }
function ChevronIcon() { return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m9 5 7 7-7 7" /></svg>; }
function CloseIcon() { return <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg>; }
