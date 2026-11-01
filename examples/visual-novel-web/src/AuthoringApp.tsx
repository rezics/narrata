import { useCallback, useEffect, useRef, useState } from "react";
import project from "../../../products/visual-novel-demo/project.json";
import mainPackage from "../../../products/visual-novel-demo/packages/main.json";
import { PlayerSurface } from "./App.js";
import { openDemoHost, type DemoHost, type VnCandidateSnapshot } from "./player-host.js";
import type { VnAuthoring } from "@rezics/narrata/vn/authoring";

type SpeakerKey = "speaker.mira" | "speaker.tomas";
type DraftValues = { dialogue: string; speaker: SpeakerKey };
type VnDrafts = ReturnType<VnAuthoring["createVnDrafts"]>;
type VnPreview = ReturnType<VnDrafts["preview"]>;
type PreviewLifecycleEvidence = {
  readonly artifactId: string;
  readonly snapshotId: string;
  status: "checking" | "opening" | "preparing" | "ready" | "failed" | "closed";
  closeStarted: boolean;
  readerClosed: boolean;
  candidateFreed: boolean;
  draftsFreed: boolean;
  aborts: number;
  releases: number;
  installed: boolean;
};
type PreviewJob = {
  readonly version: number;
  canceled: boolean;
  opening: boolean;
  candidateBytes?: VnCandidateSnapshot;
  candidate?: VnPreview;
  drafts?: VnDrafts;
  host?: DemoHost;
  evidence?: PreviewLifecycleEvidence;
  disposal?: Promise<void>;
};
type ActivePreview = { readonly host: DemoHost; readonly job: PreviewJob; readonly dispose: () => Promise<void>; readonly artifactId: string; readonly snapshotId: string };

const dialogueKey = "dialogue.mira.opening";
const speakerKeys: readonly SpeakerKey[] = ["speaker.mira", "speaker.tomas"];

export default function AuthoringApp() {
  const [draft, setDraft] = useState<DraftValues>({ dialogue: "", speaker: "speaker.mira" });
  const [preview, setPreview] = useState<ActivePreview>();
  const [message, setMessage] = useState("Loading the host draft…");
  const [busy, setBusy] = useState(false);
  const [editorOpen, setEditorOpen] = useState(true);
  const [revision, setRevision] = useState(0);
  const generation = useRef(0);
  const pendingJob = useRef<PreviewJob | undefined>(undefined);
  const livePreview = useRef<ActivePreview | undefined>(undefined);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const pendingDisposal = useRef<Promise<void> | undefined>(undefined);
  const lifecycle = useRef<PreviewLifecycleEvidence[]>([]);

  const invalidateWork = useCallback((): number => {
    generation.current++;
    if (timer.current !== undefined) {
      clearTimeout(timer.current);
      timer.current = undefined;
    }
    const job = pendingJob.current;
    pendingJob.current = undefined;
    if (job) pendingDisposal.current = disposeJob(job);
    return generation.current;
  }, []);

  const compile = useCallback(async (values: DraftValues, version: number) => {
    if (!values.dialogue.trim()) {
      if (generation.current === version) {
        setMessage("Add dialogue text to build a preview. Your last valid preview stays open.");
        setBusy(false);
      }
      return;
    }
    const job: PreviewJob = { version, canceled: false, opening: false };
    pendingJob.current = job;
    setBusy(true);
    setMessage("Checking this draft and preparing its preview…");
    let authoring: VnAuthoring | undefined;
    try {
      const { loadVnAuthoring } = await import("@rezics/narrata/vn/authoring");
      if (!isCurrent(job)) return;
      authoring = await loadVnAuthoring();
      if (!isCurrent(job)) return;
      const manifest: unknown = project;
      const source = structuredClone(mainPackage);
      const dialogue = source.graphs.start.nodes["mira-opening"].data;
      dialogue.line.unit.key = dialogueKey;
      dialogue.speaker = { provider: "host", key: values.speaker };
      const split = authoring.splitVnSource(manifest, [[project.product.entry.package, source]]);
      if (!split.complete || split.diagnostics.some(item => item.severity === "error")) {
        throw new Error(formatDiagnostics(split.diagnostics));
      }
      if (!isCurrent(job)) return;
      job.drafts = authoring.createVnDrafts();
      for (const record of split.records) job.drafts.add(record.bytes);
      const checked = job.drafts.check();
      if (!checked.complete || checked.diagnostics.some(item => item.severity === "error")) {
        throw new Error(formatDiagnostics(checked.diagnostics));
      }
      if (!isCurrent(job)) return;
      job.candidate = job.drafts.preview();
      const candidate = job.candidate;
      if (!candidate.compiled || !candidate.report.artifact_id) {
        throw new Error(formatDiagnostics(candidate.report.diagnostics));
      }
      const manifestBytes = candidate.manifest()?.slice();
      if (!manifestBytes) throw new Error("The checked preview did not include a manifest.");
      const objects = new Map<string, Uint8Array>();
      for (const id of candidate.report.objects) {
        const bytes = candidate.object(id);
        if (!bytes) throw new Error(`The checked preview omitted program object ${id}.`);
        objects.set(id, bytes.slice());
      }
      job.candidateBytes = { artifactId: candidate.report.artifact_id, manifest: manifestBytes, objects };
      const snapshotId = await textRevision(values.dialogue, values.speaker, version);
      if (!isCurrent(job)) return;
      job.evidence = {
        artifactId: job.candidateBytes.artifactId,
        snapshotId,
        status: "opening",
        closeStarted: false,
        readerClosed: false,
        candidateFreed: false,
        draftsFreed: false,
        aborts: 0,
        releases: 0,
        installed: false,
      };
      lifecycle.current.push(job.evidence);
      if (lifecycle.current.length > 8) lifecycle.current.shift();
      if (!isCurrent(job)) return;
      job.opening = true;
      let host: DemoHost;
      try {
        host = await openDemoHost({
          candidate: job.candidateBytes,
          contentDrafts: new Map([[dialogueKey, { text: values.dialogue, revision: snapshotId }]]),
        });
      } finally { job.opening = false; }
      job.host = host;
      if (!isCurrent(job)) {
        await disposeJob(job);
        return;
      }
      if (job.evidence) job.evidence.status = "preparing";
      await host.controller.start();
      if (!isCurrent(job)) {
        await disposeJob(job);
        return;
      }
      const state = host.controller.getSnapshot();
      if (state.status !== "ready") {
        const detail = state.failure?.kind === "resource"
          ? "A scene resource could not be loaded. Check the connection and refresh the preview."
          : "The draft could not be opened. Check it and refresh the preview.";
        throw new Error(detail);
      }
      const active: ActivePreview = {
        host,
        job,
        artifactId: job.candidateBytes.artifactId,
        snapshotId,
        dispose: () => disposeJob(job),
      };
      const previous = livePreview.current;
      livePreview.current = active;
      pendingJob.current = undefined;
      if (job.evidence) {
        job.evidence.status = "ready";
        job.evidence.installed = true;
      }
      setPreview(active);
      setMessage("Preview updated. Changes stay in this temporary reading session.");
      if (previous) await previous.dispose();
    } catch (error) {
      const wasCanceled = job.canceled;
      await disposeJob(job);
      if (job.evidence && !wasCanceled) job.evidence.status = "failed";
      if (pendingJob.current === job) pendingJob.current = undefined;
      if (generation.current === version) {
        setMessage(error instanceof Error ? error.message : "The draft could not be previewed. Check the dialogue and try again.");
      }
    } finally {
      if (pendingJob.current === job) pendingJob.current = undefined;
      if (generation.current === version) setBusy(false);
    }

    function isCurrent(value: PreviewJob): boolean {
      return !value.canceled && generation.current === value.version;
    }
  }, []);

  const closePreview = useCallback(async () => {
    invalidateWork();
    const pendingClose = pendingDisposal.current;
    const active = livePreview.current;
    livePreview.current = undefined;
    setPreview(undefined);
    setBusy(false);
    setMessage("Preview closed. Refresh the preview when you are ready to continue.");
    await Promise.all([active?.dispose(), pendingClose]);
    if (pendingDisposal.current === pendingClose) pendingDisposal.current = undefined;
  }, [invalidateWork]);

  useEffect(() => {
    let active = true;
    void fetch("/content/en/host/dialogue.mira.opening.txt").then(response => {
      if (!response.ok) throw new Error("The host dialogue is unavailable.");
      return response.text();
    }).then(dialogue => {
      if (!active) return;
      const initial: DraftValues = { dialogue, speaker: "speaker.mira" };
      setDraft(initial);
      setMessage("Edit the opening line or speaker to create a checked preview.");
      const version = ++generation.current;
      void compile(initial, version);
    }).catch(() => {
      if (active) setMessage("The opening dialogue could not be loaded from the host content. Reload this page to try again.");
    });
    return () => {
      active = false;
      invalidateWork();
      const current = livePreview.current;
      livePreview.current = undefined;
      void current?.dispose();
    };
  }, [compile, invalidateWork]);

  useEffect(() => {
    if (revision === 0 || !draft.dialogue.trim()) return;
    const version = generation.current;
    timer.current = setTimeout(() => {
      timer.current = undefined;
      void compile(draft, version);
    }, 260);
    return () => {
      if (timer.current !== undefined) {
        clearTimeout(timer.current);
        timer.current = undefined;
      }
    };
  }, [draft, revision, compile]);

  useEffect(() => {
    if (!import.meta.env.DEV) return;
    window.__vnAuthorHooks = {
      candidate: () => livePreview.current?.artifactId,
      snapshot: () => livePreview.current?.snapshotId,
      jobs: () => lifecycle.current.map(item => ({ ...item })),
      async closePreview() { await closePreview(); },
    };
    return () => { delete window.__vnAuthorHooks; };
  }, [closePreview]);

  const change = (next: DraftValues) => {
    invalidateWork();
    setDraft(next);
    setRevision(value => value + 1);
    if (!next.dialogue.trim()) {
      setBusy(false);
      setMessage("Add dialogue text to build a preview. Your last valid preview stays open.");
    } else {
      setBusy(true);
      setMessage("Checking this draft and preparing its preview…");
    }
  };

  const refresh = () => {
    const version = invalidateWork();
    void compile(draft, version);
  };

  return (
    <>
      {preview ? <PlayerSurface host={preview.host} authorMode /> : <main className="boot-state"><span className="loader" aria-hidden="true" /><p>{message}</p></main>}
      {editorOpen ? <section className="author-panel" aria-label="Story draft editor">
        <header className="author-heading"><div><p className="author-kicker">STORY DRAFT</p><h1>Edit the opening</h1></div><div className="author-heading-actions"><button className="author-hide" type="button" onClick={() => setEditorOpen(false)}>Hide</button><a className="author-close" href="/" aria-label="Close editor">×</a></div></header>
        <label htmlFor="draft-dialogue">Dialogue</label>
        <textarea id="draft-dialogue" rows={4} value={draft.dialogue} onChange={event => change({ ...draft, dialogue: event.currentTarget.value })} />
        <label htmlFor="draft-speaker">Speaker</label>
        <select id="draft-speaker" value={draft.speaker} onChange={event => {
          const value = event.currentTarget.value;
          if (isSpeakerKey(value)) change({ ...draft, speaker: value });
        }}>
          <option value="speaker.mira">Mira</option><option value="speaker.tomas">Tomas</option>
        </select>
        <p className={`author-status${busy ? " is-busy" : ""}`} role="status" aria-live="polite">{message}</p>
        <div className="author-actions">
          <button className="author-retry" type="button" disabled={busy || !draft.dialogue.trim()} onClick={refresh}>Refresh preview</button>
          <button className="author-close-preview" type="button" disabled={!preview && !pendingJob.current} onClick={() => { void closePreview(); }}>Close preview</button>
        </div>
        <p className="author-note">The preview uses its own temporary progress. Your saved reading position stays as it was.</p>
      </section> : <button className="author-reopen" type="button" onClick={() => setEditorOpen(true)}>Edit draft</button>}
    </>
  );
}

async function disposeJob(job: PreviewJob): Promise<void> {
  if (job.disposal) return job.disposal;
  job.canceled = true;
  if (!job.host && job.opening) return Promise.resolve();
  const host = job.host;
  if (job.evidence) job.evidence.closeStarted = true;
  const closing = host?.close() ?? Promise.resolve();
  job.disposal = (async () => {
    try {
      await closing;
    } finally {
      job.candidate?.free();
      job.drafts?.free();
      delete job.candidate;
      delete job.drafts;
      delete job.candidateBytes;
      delete job.host;
      if (job.evidence) {
        job.evidence.status = "closed";
        job.evidence.readerClosed = Boolean(host);
        job.evidence.candidateFreed = true;
        job.evidence.draftsFreed = true;
        job.evidence.aborts = host?.metrics.aborts ?? 0;
        job.evidence.releases = host?.metrics.releases ?? 0;
      }
    }
  })();
  return job.disposal;
}

function isSpeakerKey(value: string): value is SpeakerKey {
  return speakerKeys.some(key => key === value);
}

function formatDiagnostics(diagnostics: readonly { readonly severity: string; readonly message: string; readonly pointer: string }[]): string {
  const errors = diagnostics.filter(item => item.severity === "error");
  return errors.length
    ? errors.map(item => `${item.pointer || "Draft"}: ${item.message}`).join(" ")
    : "The VN source could not be checked. Review the draft and try again.";
}

async function textRevision(text: string, speaker: SpeakerKey, version: number): Promise<string> {
  const bytes = new TextEncoder().encode(`${version}\0${speaker}\0${text}`);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return `draft-${[...new Uint8Array(digest)].map(value => value.toString(16).padStart(2, "0")).join("")}`;
}
