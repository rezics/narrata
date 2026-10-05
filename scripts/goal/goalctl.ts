// Goal worker-process control for Narrata: exclusive claims, dispatch, waits, check slots, scope, merge and archive.
// Ported from REZICS's goalctl for a Windows host without Docker. The manager is the only caller of the
// state-changing commands; docs/goals/README.md describes the practice around them.
import { dlopen, FFIType, ptr } from 'bun:ffi';
import { spawn, spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { closeSync, copyFileSync, existsSync, mkdirSync, openSync, readdirSync, readFileSync, readlinkSync, readSync, realpathSync,
  renameSync, rmSync, statfsSync, statSync, writeFileSync } from 'node:fs';
import { devNull, homedir } from 'node:os';
import { basename, dirname, isAbsolute, join, posix, relative, resolve, sep } from 'node:path';

export type State = 'running' | 'exited' | 'conflict' | 'merged' | 'stopped' | 'verified' | 'cancelled';
export type Engine = 'claude' | 'sonnet' | 'fable' | 'codex' | 'grok';
export interface Brief {
  id: string; title: string; effort: string; engine: Engine; cases: string[]; paths: string[]; shared: string[];
  depends: string[];
  /** REZICS reserved SQL migration ranges here. Narrata has none, so a brief may keep `migrations: []` but no more. */
  migrations?: string[];
  /** A shared worktree name: tasks naming the same one work concurrently in one tree, branch and Cargo target. */
  worktree?: string;
}
export interface Attempt {
  n: number; effort: string; engine: Engine; pid: number;
  /** The process's start mark when it was spawned, so a reused PID never counts as the worker. */
  pidMark?: number;
  session: string; output: string; lastMessage?: string; startedAt: string; endedAt?: string;
}
export interface Task extends Omit<Brief, 'worktree' | 'migrations'> {
  /** Repository-relative brief path with forward slashes. */
  brief: string; worktree: string; branch: string; base: string; state: State; attempts: Attempt[];
  /** Set when the task works in a shared worktree. */
  worktreeName?: string;
  goal: string; mergedCommit?: string; closedAt?: string;
}
/** One running Goal. Its intent and areas live in `docs/goals/<slug>/GOAL.md`; the ledger keeps only runtime state. */
export interface GoalRecord {
  manager: string; startedAt: string; closedAt?: string; archive?: string;
  /** The `main` commit whose documentation the manager last saw, to list maintainer edits at the next dispatch. */
  seenHead?: string;
}
export interface Ledger {
  startedAt?: string;
  goals?: Record<string, GoalRecord>;
  /** IDs handed out by `new` and not yet dispatched, with the Goal each belongs to. */
  reserved?: Record<string, string>;
  /** The highest ID ever used, so IDs stay unique after closed Goals leave the ledger. */
  lastId?: number;
  tasks: Record<string, Task>;
}

// ---------------------------------------------------------------------------------------------------------------
// Engines. Which engine, model and effort a task gets is the manager's decision (docs/goals/manager.md), not a rule
// here. Whether an engine's CLI is signed in is a runtime fact: dispatch and resume probe it (`preflight`).

const CLAUDE_EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max'] as const;
export const ENGINE_EFFORTS: Record<Engine, readonly string[]> = {
  claude: CLAUDE_EFFORTS, sonnet: CLAUDE_EFFORTS, fable: CLAUDE_EFFORTS,
  codex: [...CLAUDE_EFFORTS, 'ultra'],
  grok: ['low', 'medium', 'high'],
};
export const ENGINES = Object.keys(ENGINE_EFFORTS) as Engine[];
const isEngine = (value: string | undefined): value is Engine => !!value && (ENGINES as string[]).includes(value);
export const DEFAULT_ENGINE: Engine = isEngine(process.env.GOAL_ENGINE) ? process.env.GOAL_ENGINE : 'claude';
const engineOf = (item: { engine?: Engine }): Engine => item.engine ?? 'claude';
const effortsOf = (engine: Engine): readonly string[] => ENGINE_EFFORTS[engine] ?? [];
export const isClaudeCode = (engine: Engine): boolean => engine === 'claude' || engine === 'sonnet' || engine === 'fable';
type Family = 'claude' | 'codex' | 'grok';
const familyOf = (engine: Engine): Family => (isClaudeCode(engine) ? 'claude' : engine as Family);

export const MODEL = 'claude-opus-5-5';
export const FABLE_MODEL = 'claude-fable-5-1';
export const CODEX_MODEL = 'gpt-6.1-sol';
export const GROK_MODEL = 'grok-4.7';

/** Model IDs, read at call time so an environment override applies to the next dispatch.
 * Sonnet: REZICS found Claude Code 2.1.284 the first release that accepts claude-sonnet-5-5. */
export function modelOf(engine: Engine): string {
  switch (engine) {
    case 'claude': return MODEL;
    case 'sonnet': return process.env.GOAL_SONNET_MODEL || 'claude-sonnet-5-5';
    case 'fable': return FABLE_MODEL;
    case 'codex': return process.env.GOAL_CODEX_MODEL || CODEX_MODEL;
    case 'grok': return process.env.GOAL_GROK_MODEL || GROK_MODEL;
  }
}

export const codexHome = (): string => process.env.GOAL_CODEX_HOME || join(homedir(), '.codex');

/** The executable for a CLI family: `GOAL_<FAMILY>_COMMAND` may name another binary or a JSON argv prefix
 * (`["bun","fake-claude.ts"]`); tests use it to run a stand-in worker. */
export function cliCommand(family: Family): string[] {
  const override = process.env[`GOAL_${family.toUpperCase()}_COMMAND`]?.trim();
  if (!override) return [family];
  if (override.startsWith('[')) return JSON.parse(override) as string[];
  return [override];
}

// Windows caps a command line at 32,767 characters. Claude Code and Codex read the prompt from stdin, which also
// avoids quoting; the Grok CLI takes it only as an argument.
const COMMAND_LINE_LIMIT = 30_000;
export const promptOnStdin = (engine: Engine): boolean => engine !== 'grok';

// Workers run in bypass permission mode, as the manager does, and accept no inbound instructions; the manager
// changes a worker's instructions only by stopping and resuming it.
export function launchCommand(options: { id: string; effort: string; session: string; prompt: string;
  resume: boolean; engine?: Engine; worktree?: string; lastMessage?: string }): [string, string[]] {
  const { id, effort, session, prompt, resume } = options;
  const engine = options.engine ?? 'claude';
  const [program, ...prefix] = cliCommand(familyOf(engine));
  const model = modelOf(engine);
  let args: string[];
  if (engine === 'grok') {
    args = ['-p', prompt, '-m', model, '--reasoning-effort', effort, '--permission-mode', 'bypassPermissions',
      '--no-subagents', '--output-format', 'json', '--cwd', options.worktree ?? '.', ...(resume ? ['-r', session] : [])];
  } else if (engine === 'codex') {
    // A service tier is passed only when the manager sets one; the account's default applies otherwise.
    const tier = process.env.GOAL_CODEX_SERVICE_TIER?.trim();
    const common = ['-m', model, '-c', `model_reasoning_effort=${effort}`, ...(tier ? ['-c', `service_tier="${tier}"`] : []),
      '--dangerously-bypass-approvals-and-sandbox', '--json', '-o', options.lastMessage ?? devNull];
    // `-` reads the prompt from stdin.
    args = resume ? ['exec', 'resume', session, ...common, '-'] : ['exec', ...common, '-C', options.worktree ?? '.', '-'];
  } else {
    // No positional prompt: `claude -p` reads it from stdin.
    args = ['-p', '--model', model, '--effort', effort, '--dangerously-skip-permissions',
      ...(resume ? ['--resume', session] : ['--session-id', session]), '-n', id.toLowerCase(), '--output-format', 'json'];
  }
  const all = [...prefix, ...args];
  const length = [program!, ...all].reduce((sum, arg) => sum + arg.length + 3, 0);
  if (length > COMMAND_LINE_LIMIT) {
    throw new Error(`${engine} command line is ${length} characters, past the Windows limit; shorten the message`);
  }
  return [program!, all];
}

/** Variables that tie a process to the manager's own Claude Code session. A worker is a session of its own: it must
 * not pass for the manager's child, reuse its effort or reach its messaging socket. */
export const MANAGER_SESSION_ENV = ['CLAUDECODE', 'CLAUDE_PID', 'CLAUDE_EFFORT', 'CLAUDE_CODE_SESSION_ID',
  'CLAUDE_CODE_HOST_SESSION_ID', 'CLAUDE_CODE_CHILD_SESSION', 'CLAUDE_CODE_SSE_PORT', 'CLAUDE_CODE_ENTRYPOINT',
  'CLAUDE_CODE_MESSAGING_SOCKET', 'CLAUDE_CODE_MESSAGING_TOKEN'] as const;

export function workerEnv(base: Record<string, string | undefined>, extra: Record<string, string> = {}): Record<string, string> {
  const env: Record<string, string> = {};
  for (const [key, value] of Object.entries(base)) {
    if (value !== undefined && !(MANAGER_SESSION_ENV as readonly string[]).includes(key.toUpperCase())) env[key] = value;
  }
  return { ...env, ...extra };
}

/** Per-worktree Cargo targets are the default. `GOAL_CARGO_TARGET_DIR` shares one (relative to the main checkout):
 * dependencies build once, but every cargo command on the host then queues on that directory's lock. */
function cargoEnv(): Record<string, string> {
  const shared = process.env.GOAL_CARGO_TARGET_DIR?.trim();
  return shared ? { CARGO_TARGET_DIR: isAbsolute(shared) ? shared : join(repoRoot(), shared) } : {};
}

const AUTH_HELP: Record<Family, string> = { claude: 'claude auth login', codex: 'codex login', grok: 'grok login' };
const AUTH_PROBE: Record<Family, string[]> = { claude: ['auth', 'status', '--json'], codex: ['login', 'status'], grok: ['models'] };
const NOT_SIGNED_IN = /OAuth session expired|Failed to authenticate|not (?:signed|logged) in|not authenticated|authentication_error|invalid (?:api|x-api)[- ]key|\b401\b/i;

/** Reads an engine CLI's sign-in probe. Undefined when it is signed in or the probe says nothing either way. */
export function authProblem(engine: Engine, probe: { status: number | null; stdout?: string; stderr?: string;
  error?: { code?: string; message?: string } }): string | undefined {
  const family = familyOf(engine);
  if (probe.error?.code === 'ENOENT') return `${cliCommand(family)[0]} is not on PATH`;
  if (probe.error) return undefined;
  const text = `${probe.stdout ?? ''}\n${probe.stderr ?? ''}`;
  if (family === 'claude') {
    const json = /\{[\s\S]*\}/.exec(probe.stdout ?? '')?.[0];
    try {
      const status = JSON.parse(json ?? '') as { loggedIn?: boolean; authMethod?: string };
      if (status.loggedIn === false) return `claude is not signed in (auth method ${status.authMethod ?? 'none'})`;
      if (status.loggedIn === true) return undefined;
    } catch { /* not JSON */ }
  }
  if (NOT_SIGNED_IN.test(text)) return `${family} is not signed in: ${text.trim().split('\n')[0]}`;
  if (family === 'codex' && probe.status !== 0) return `codex login status failed: ${text.trim().split('\n')[0]}`;
  return undefined;
}

const preflightCache = new Map<Family, string | undefined>();

/** A cheap sign-in probe per CLI family (no model call). The maintainer signs in; goalctl never does. */
function preflight(engine: Engine, cwd: string): void {
  const family = familyOf(engine);
  if (!preflightCache.has(family)) {
    const [program, ...prefix] = cliCommand(family);
    const probe = spawnSync(program!, [...prefix, ...AUTH_PROBE[family]], { cwd, encoding: 'utf8', timeout: 30_000,
      windowsHide: true, env: workerEnv(process.env) });
    if (probe.error && (probe.error as NodeJS.ErrnoException).code !== 'ENOENT') {
      console.error(`warning: could not check ${family} sign-in: ${probe.error.message}`);
    }
    preflightCache.set(family, authProblem(engine, { status: probe.status, stdout: probe.stdout, stderr: probe.stderr,
      error: probe.error as NodeJS.ErrnoException | undefined }));
  }
  const problem = preflightCache.get(family);
  if (problem) {
    throw new Error(`${engine}: ${problem}. The maintainer signs in with \`${AUTH_HELP[family]}\`; `
      + 'choose another engine or pass --skip-preflight');
  }
}

/** What a failed attempt's output says about its CLI, for `wait`. */
export function failureHint(engine: Engine, text: string): string | undefined {
  const family = familyOf(engine);
  if (NOT_SIGNED_IN.test(text)) {
    return `AUTH: ${family} is not signed in or its session expired; the maintainer signs in with \`${AUTH_HELP[family]}\`,`
      + ' then resume the task';
  }
  if (/rate[ -]?limit|usage limit|quota|limit reached|\b429\b/i.test(text)) {
    return `LIMIT: ${family} reports a usage limit; back off, or resume on another engine with --engine <e> --fresh`;
  }
  return undefined;
}

// ---------------------------------------------------------------------------------------------------------------
// Briefs and claims.

/** The leading `---` block of a brief or Goal file as flat `key: value` fields. `#` at the start of a line or after a
 * space begins a comment. A key with no value may be followed by `- item` lines, read as the list `[item, ...]`. */
export function parseFrontmatter(text: string): Record<string, string> | undefined {
  const block = /^\uFEFF?---\r?\n([\s\S]*?)\r?\n---\r?\n/.exec(text);
  if (!block) return undefined;
  const fields: Record<string, string> = {};
  const items: Record<string, string[]> = {};
  let last: string | undefined;
  for (const line of block[1]!.split(/\r?\n/)) {
    const clean = line.replace(/(?:^|\s+)#.*$/, '').trim();
    if (!clean) continue;
    const item = /^-\s+(.+)$/.exec(clean);
    if (item && last !== undefined && (fields[last] === '' || items[last])) {
      (items[last] ??= []).push(item[1]!.trim());
      continue;
    }
    const at = clean.indexOf(':');
    if (at < 1) throw new Error(`Bad frontmatter line: ${line}`);
    last = clean.slice(0, at).trim();
    fields[last] = clean.slice(at + 1).trim();
  }
  for (const [key, list] of Object.entries(items)) fields[key] = `[${list.join(', ')}]`;
  return fields;
}

function listField(fields: Record<string, string>, key: string): string[] {
  const value = fields[key];
  if (!value) return [];
  if (!value.startsWith('[') || !value.endsWith(']')) throw new Error(`${key} must be a [a, b] list`);
  return value.slice(1, -1).split(',').map(item => item.trim()).filter(Boolean);
}

export function parseBrief(text: string): Brief {
  const fields = parseFrontmatter(text);
  if (!fields) throw new Error('Brief needs a leading --- frontmatter block');
  const list = (key: string): string[] => listField(fields, key);
  return {
    id: fields.id ?? '', title: fields.title ?? '', effort: fields.effort ?? 'medium',
    engine: (fields.engine || DEFAULT_ENGINE) as Engine,
    cases: list('cases'), paths: list('paths'), shared: list('shared'), depends: list('depends'),
    ...fields.migrations !== undefined ? { migrations: list('migrations') } : {},
    ...fields.worktree ? { worktree: fields.worktree } : {},
  };
}

const TASK_ID_FORMAT = /^G-\d{3,}$/;

export function validateBrief(brief: Brief): string[] {
  const errors: string[] = [];
  if (!TASK_ID_FORMAT.test(brief.id)) errors.push(`id must look like G-001: ${brief.id || '(missing)'}`);
  if (!brief.title) errors.push('title is required');
  const engine = brief.engine ?? DEFAULT_ENGINE;
  if (!isEngine(engine)) errors.push(`engine must be one of ${ENGINES.join(', ')}: ${engine}`);
  else if (!effortsOf(engine).includes(brief.effort)) {
    errors.push(`${engine} effort must be one of ${effortsOf(engine).join(', ')}: ${brief.effort}`);
  }
  for (const id of brief.cases) if (!/^[A-Z][A-Z0-9]*\d$/.test(id)) errors.push(`bad case ID: ${id}`);
  for (const path of brief.paths) {
    if (path.includes('\\')) errors.push(`path claim must use forward slashes: ${path}`);
    else if (isAbsolute(path) || /^[A-Za-z]:/.test(path) || path.split('/').includes('..') || path.startsWith('.temp/')) {
      errors.push(`path claim must be repository-relative outside .temp: ${path}`);
    }
  }
  if (brief.migrations?.length) {
    errors.push('migrations: Narrata has no numbered migrations to reserve; claim the files under paths and '
      + 'reserve numbers (ADRs, frozen corpora) with shared slots such as adr:0012');
  }
  for (const id of brief.depends) if (!TASK_ID_FORMAT.test(id)) errors.push(`bad dependency: ${id}`);
  if (brief.worktree !== undefined && !/^[a-z][a-z0-9-]{1,40}$/.test(brief.worktree)) {
    errors.push(`worktree must be a lower-case name: ${brief.worktree}`);
  }
  return errors;
}

function literalEnds(segment: string): [string, string] {
  const first = segment.search(/[*?[{]/);
  if (first < 0) return [segment, segment];
  let last = -1;
  for (let at = 0; at < segment.length; at++) if ('*?[]{}'.includes(segment[at]!)) last = at;
  return [segment.slice(0, first), segment.slice(last + 1)];
}

function segmentRegex(segment: string): RegExp {
  const body = segment.replace(/[.+^$()|\\]/g, '\\$&').replace(/\*+/g, '[^/]*').replace(/\?/g, '[^/]');
  return new RegExp(`^${body}$`);
}

// A bracketed directory name such as `[locale]` is claimed as `[[]locale]` or `\\[locale\\]`; map those escaped
// brackets to stand-in characters so they compare as literals.
function literalBrackets(segment: string): string {
  if (/^\[\[?(?:\.\.\.)?[A-Za-z][\w-]*\]\]?$/.test(segment)) {
    return segment.replaceAll('[', '\u0001').replaceAll(']', '\u0002');
  }
  const escaped = segment.replace(/\[\[\]|\\\[/g, '\u0001').replace(/\[\]\]|\\\]/g, '\u0002');
  return escaped.includes('[') ? escaped : escaped.replaceAll(']', '\u0002');
}

// Two single path segments can match a common name. Exact for literals and for a literal against `*`/`?`; for two
// wildcard segments only the literal prefix and suffix are compared, which may report overlap where none exists.
function segmentsIntersect(rawA: string, rawB: string): boolean {
  const a = literalBrackets(rawA);
  const b = literalBrackets(rawB);
  const wildA = /[*?[{]/.test(a);
  const wildB = /[*?[{]/.test(b);
  if (!wildA && !wildB) return a === b;
  if (!wildA || !wildB) {
    const [literal, glob] = wildA ? [b, a] : [a, b];
    return /[[{]/.test(glob) || segmentRegex(glob).test(literal);
  }
  const [prefixA, suffixA] = literalEnds(a);
  const [prefixB, suffixB] = literalEnds(b);
  return (prefixA.startsWith(prefixB) || prefixB.startsWith(prefixA))
    && (suffixA.endsWith(suffixB) || suffixB.endsWith(suffixA));
}

function globsIntersect(a: string[], i: number, b: string[], j: number): boolean {
  if (i === a.length && j === b.length) return true;
  if (a[i] === '**') return globsIntersect(a, i + 1, b, j) || (j < b.length && globsIntersect(a, i, b, j + 1));
  if (b[j] === '**') return globsIntersect(a, i, b, j + 1) || (i < a.length && globsIntersect(a, i + 1, b, j));
  if (i === a.length || j === b.length) return false;
  return segmentsIntersect(a[i]!, b[j]!) && globsIntersect(a, i + 1, b, j + 1);
}

/** Claims overlap when some file path could match both globs (segment-wise, `**` spanning any number of segments).
 * Uncertain wildcard pairs count as overlap. */
export function pathsOverlap(a: string, b: string): boolean {
  return globsIntersect(a.split('/'), 0, b.split('/'), 0);
}

const HOLDING: State[] = ['running', 'exited', 'conflict', 'merged', 'stopped'];
const CLOSED: State[] = ['verified', 'cancelled'];

export function claimConflicts(brief: Brief, tasks: Task[]): string[] {
  const conflicts: string[] = [];
  for (const task of tasks) {
    if (task.id === brief.id || !HOLDING.includes(task.state)) continue;
    for (const id of brief.cases) if (task.cases.includes(id)) conflicts.push(`case ${id} is held by ${task.id}`);
    for (const path of brief.paths) {
      for (const held of task.paths) if (pathsOverlap(path, held)) conflicts.push(`path ${path} overlaps ${task.id} ${held}`);
    }
    for (const token of brief.shared) if (task.shared.includes(token)) conflicts.push(`shared ${token} is held by ${task.id}`);
  }
  return conflicts;
}

export function outOfScope(files: string[], patterns: string[]): string[] {
  const globs = patterns.map(pattern => new Bun.Glob(pattern));
  return files.filter(file => !globs.some(glob => glob.match(file)));
}

// Several Goals run at once, one manager each. A Goal is the directory docs/goals/<slug>/: GOAL.md states it and lists
// its areas, state.md keeps its checkpoint, and tasks/ holds its open briefs. The directory moves to archive/goals/
// when the Goal closes; docs/goals/{README,manager,worker}.md are the program and stay.
export const GOALS_DIR = 'docs/goals';
export const ARCHIVE_DIR = 'archive/goals';
const GOAL_SLUG = /^[a-z][a-z0-9-]{1,40}$/;

export const validGoalSlug = (slug: string): boolean => GOAL_SLUG.test(slug) && slug !== 'tasks';
export const goalFile = (slug: string): string => `${GOALS_DIR}/${slug}/GOAL.md`;
export const goalBriefFile = (slug: string, id: string): string => `${GOALS_DIR}/${slug}/tasks/${id}.md`;

/** The Goal a repository-relative brief path belongs to; undefined outside `docs/goals/<slug>/tasks/`. */
export function goalOfBriefPath(path: string): string | undefined {
  const slug = /^docs\/goals\/([^/]+)\/tasks\/G-\d{3,}\.md$/.exec(path.replaceAll('\\', '/'))?.[1];
  return slug && validGoalSlug(slug) ? slug : undefined;
}

/** A Goal's areas: coarse path globs that other Goals' briefs may not claim. Brace and bracket globs compare as
 * overlapping almost everything (`pathsOverlap` errs on the safe side), so list each directory instead. */
export function goalAreas(goalText: string): string[] {
  const fields = parseFrontmatter(goalText);
  return fields ? listField(fields, 'areas') : [];
}

/** Claims that fall in another active Goal's areas. Paths in no Goal's areas belong to whoever claims them. */
export function areaConflicts(paths: string[], goal: string | undefined, areas: Record<string, string[]>): string[] {
  const conflicts: string[] = [];
  for (const [other, globs] of Object.entries(areas)) {
    if (other === goal) continue;
    for (const path of paths) {
      for (const area of globs) if (pathsOverlap(path, area)) conflicts.push(`path ${path} lies in Goal ${other}'s area ${area}`);
    }
  }
  return conflicts;
}

/** The next task ID after every one in use, reserved, archived or recorded as the high-water mark. */
export function nextTaskId(ids: Iterable<string>, lastId = 0): string {
  let highest = lastId;
  for (const id of ids) {
    const number = /^G-(\d{3,})$/.exec(id)?.[1];
    if (number) highest = Math.max(highest, Number(number));
  }
  return `G-${String(highest + 1).padStart(3, '0')}`;
}

// Task IDs are history: they belong in commit messages, branches, the ledger and archive/goals. In the tree they become
// dangling pointers once a brief is archived, and files named after tasks organize code by when it was written instead
// of what it covers (REZICS accumulated about 600 such files before its merge check).
export const HISTORY_EXEMPT = ['GOAL.md', `${GOALS_DIR}/**`, 'scripts/goal/**', 'archive/**'];
const TASK_ID = /\bG-\d{3,}\b/g;
const taskSegment = (path: string): boolean => path.split('/').some(segment => /^g-\d{3,}(?!\d)/i.test(segment));

export interface ChangedFile {
  path: string; status: 'A' | 'M' | 'D' | 'R';
  /** The previous path of a rename. */
  from?: string;
  before?: string; after?: string;
}

function idCounts(text: string): Map<string, number> {
  const counts = new Map<string, number>();
  for (const id of text.match(TASK_ID) ?? []) counts.set(id, (counts.get(id) ?? 0) + 1);
  return counts;
}

/** What a change adds that names a task: new task-named files and task IDs a file did not carry before.
 * Existing names and mentions may stay or move within their file; renaming them away is always allowed. */
export function historyIntroductions(files: readonly ChangedFile[]): string[] {
  const exempt = HISTORY_EXEMPT.map(pattern => new Bun.Glob(pattern));
  const found: string[] = [];
  for (const file of files) {
    if (file.status === 'D' || exempt.some(glob => glob.match(file.path))) continue;
    const renamedFromTask = file.status === 'R' && !!file.from && taskSegment(file.from);
    if ((file.status === 'A' || file.status === 'R') && taskSegment(file.path) && !renamedFromTask) {
      found.push(`${file.path}: named after a task; name it by the capability it covers`);
    }
    if (file.after === undefined || file.after.includes('\0')) continue;
    const before = idCounts(file.before ?? '');
    const added = [...idCounts(file.after)].filter(([id, count]) => count > (before.get(id) ?? 0)).map(([id]) => id);
    if (added.length) found.push(`${file.path}: adds ${added.join(', ')}; state the reason itself instead of citing the task`);
  }
  return found;
}

/** A state-changing command on another Goal's task. GOAL_ID is set in each manager's environment. */
export function ownerRefusal(task: Pick<Task, 'id' | 'goal'>, caller = process.env.GOAL_ID): string | undefined {
  return caller && task.goal && task.goal !== caller
    ? `${task.id} belongs to Goal ${task.goal}; ask its manager (GOAL_ID is ${caller})` : undefined;
}

/** Commands heavy enough that the host runs one at a time: the PowerShell gates and benchmarks (directly or as
 * `task check:*` / `task bench:*`), browser suites (Playwright uses the fixed port 4173 and reuses a server already
 * listening there, so two runs would test each other's build) and cargo builds of the whole workspace.
 * `slot --heavy` marks any other command. */
export function isHeavyCommand(command: readonly string[]): boolean {
  const words = command.map(word => word.replaceAll('\\', '/').toLowerCase());
  if (words.some(word => /(?:^|\/)(?:check|benchmark)-[\w-]+\.ps1$/.test(word))) return true;
  const task = words.findIndex(word => /(?:^|\/)task(?:\.exe)?$/.test(word));
  if (task >= 0 && words.slice(task + 1).some(word => /^(?:check|bench):/.test(word))) return true;
  if (words.some(word => word.includes('playwright') || word === 'test:e2e')) return true;
  const cargo = words.findIndex(word => /(?:^|\/)cargo(?:\.exe)?$/.test(word));
  return cargo >= 0 && ['test', 'build', 'clippy', 'check', 'bench', 'doc', 'nextest'].includes(words[cargo + 1] ?? '')
    && words.some(word => word === '--workspace' || word === '--all');
}

/** Where a task's brief lives in its worktree; shared worktrees hold one brief per task. */
export function briefFile(task: Pick<Task, 'id'> & { shared?: boolean }): string {
  return task.shared ? `.temp/goal/brief-${task.id.toLowerCase()}.md` : '.temp/goal/brief.md';
}

const forward = (path: string): string => path.replaceAll('\\', '/');

function workerPrompt(task: Task, manager: string, engine: Engine = engineOf(task), effort = task.effort): string {
  const tree = forward(task.worktree);
  const brief = briefFile({ ...task, shared: !!task.worktreeName });
  const sharedTarget = cargoEnv().CARGO_TARGET_DIR;
  return [
    `You are Narrata Goal worker ${task.id} (${modelOf(engine)}/${effort}) of Goal ${task.goal}. Your worktree is`
      + ` ${tree} on branch ${task.branch}; work only there.`,
    `Edit, create and delete files only under ${tree} and the system temporary directory. The main checkout`
      + ` ${forward(repoRoot())} and every other worktree are read-only for you, even when a brief or handoff cites an`
      + ` absolute path there; translate such paths to ${tree}.`,
    `Read AGENTS.md, then docs/goals/worker.md, then your brief at ${brief} (all in your worktree), and follow them.`,
    'Run builds, tests and gates through `task goal -- slot -- <command>` (add `--heavy` for gates, browser suites and'
      + ' workspace-wide cargo runs) so that concurrent workers share the host; goalctl may write its bookkeeping under'
      + ' the main checkout\'s .temp/goal, which is allowed.',
    sharedTarget ? `CARGO_TARGET_DIR is ${forward(sharedTarget)}, shared with other workers: expect cargo to wait for`
      + ' its lock, and never clean it.'
      : 'Cargo builds into this worktree\'s own target/ directory; do not clean other directories.',
    `End every commit message with the trailer \`Goal: ${task.goal}\`.`,
    ...task.worktreeName ? [`Other workers share this worktree at the same time (shared worktree "${task.worktreeName}").`
      + ' Change only your claimed paths and commit only them with `git commit --only <paths>` (retry if index.lock is'
      + ' held); never reset, checkout, stash or reformat files you did not change (run rustfmt on your files only).'
      + ' Start no dev server or browser; leave browser suites to the manager unless your brief asks for them.'] : [],
    `Your Goal's manager session is "${manager}". End with the handoff that the worker protocol specifies.`,
  ].join('\n');
}

// ---------------------------------------------------------------------------------------------------------------
// Git and the repository.

function gitRaw(cwd: string, args: string[], allowFail = false,
  options: { env?: Record<string, string>; input?: string } = {}): string {
  const result = spawnSync('git', ['-c', 'core.quotePath=false', ...args], { cwd, encoding: 'utf8', input: options.input,
    maxBuffer: 256 * 1024 * 1024, windowsHide: true, env: options.env ? { ...process.env, ...options.env } : undefined });
  if (result.status !== 0 && !allowFail) throw new Error(`git ${args.join(' ')} failed:\n${result.stderr || result.error?.message}`);
  return result.status === 0 ? result.stdout : '';
}

const git = (cwd: string, args: string[], allowFail = false): string => gitRaw(cwd, args, allowFail).trim();
const nulList = (text: string): string[] => text.split('\0').filter(Boolean);

/** File paths of `git status --porcelain -z`; a rename or copy entry is followed by its source path, skipped here. */
export function parsePorcelainZ(text: string): string[] {
  const entries = text.split('\0');
  const files: string[] = [];
  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i]!;
    if (entry.length < 4) continue;
    files.push(entry.slice(3));
    if (/[RC]/.test(entry.slice(0, 2))) i++;
  }
  return files;
}

let cachedRoot: string | undefined;
/** The main checkout, from any of its worktrees. */
export function repoRoot(): string {
  return cachedRoot ??= realpathSync.native(dirname(git(process.cwd(), ['rev-parse', '--path-format=absolute', '--git-common-dir'])));
}
const stateDir = (): string => join(repoRoot(), '.temp', 'goal');
const ledgerPath = (): string => join(stateDir(), 'ledger.json');
const repoPath = (absolute: string): string => relative(repoRoot(), absolute).split(sep).join('/');

/** Commit trailers for goalctl's own commits: the Goal and, per the maintainer's attribution rule, the co-author
 * (`GOAL_CO_AUTHOR` changes it; an empty value omits it). */
export function commitTrailers(goals: Iterable<string>): string {
  const coAuthor = process.env.GOAL_CO_AUTHOR ?? 'Claude Opus 5.5 <noreply@anthropic.com>';
  return [...[...new Set(goals)].map(goal => `Goal: ${goal}`), ...coAuthor ? [`Co-Authored-By: ${coAuthor}`] : []].join('\n');
}

/** Applies `change` to the parts of a Markdown line outside inline code spans. */
function outsideCode(line: string, change: (text: string) => string): string {
  let out = '';
  let last = 0;
  for (const match of line.matchAll(/(`+)[^`]*?\1/g)) {
    out += change(line.slice(last, match.index)) + match[0];
    last = match.index! + match[0].length;
  }
  return out + change(line.slice(last));
}

/** Where a repository path went, or undefined when it stays. */
export type Mover = (path: string) => string | undefined;

/** Rewrites the relative links of a Markdown file that moves from `from` to `to` (repository paths, `/`-separated) so
 * they reach the same targets, following targets that move too. Code spans, fenced blocks, URLs and same-page anchors
 * are left alone. `task docs:check` checks archive/ as well, so archived files must keep working links. */
export function relinkMarkdown(content: string, from: string, to: string, move: Mover = () => undefined): string {
  const fromDir = posix.dirname(from);
  const toDir = posix.dirname(to);
  const retarget = (target: string): string => {
    if (/^[a-z][a-z0-9+.-]*:/i.test(target) || target.startsWith('#') || target.startsWith('/')) return target;
    const at = target.indexOf('#');
    const path = at < 0 ? target : target.slice(0, at);
    if (!path) return target;
    let decoded = path;
    try { decoded = decodeURIComponent(path); } catch { /* keep as written */ }
    const resolved = posix.normalize(posix.join(fromDir, decoded)).replace(/\/$/, '');
    if (resolved === '..' || resolved.startsWith('../')) return target;
    const moved = move(resolved);
    if (moved === undefined && fromDir === toDir) return target;
    const relativePath = posix.relative(toDir, moved ?? resolved) || '.';
    return (decoded === path ? relativePath : encodeURI(relativePath)) + (at < 0 ? '' : target.slice(at));
  };
  let fence: string | undefined;
  return content.split('\n').map(line => {
    const marker = /^\s{0,3}(`{3,}|~{3,})/.exec(line)?.[1];
    if (fence) {
      if (marker && marker[0] === fence[0] && marker.length >= fence.length) fence = undefined;
      return line;
    }
    if (marker) { fence = marker; return line; }
    const definition = /^(\s{0,3}\[[^\]]+\]:\s*<?)([^\s>]+)(.*)$/.exec(line);
    if (definition) return `${definition[1]}${retarget(definition[2]!)}${definition[3]}`;
    return outsideCode(line, text => text.replace(/(!?\[[^\]]*\]\(\s*<?)([^)\s>]+)/g,
      (_, head: string, target: string) => `${head}${retarget(target)}`));
  }).join('\n');
}

/** A fence longer than any backtick run in `text`, so worker output cannot close it early. */
function fenced(text: string): string {
  const longest = Math.max(2, ...[...text.matchAll(/`+/g)].map(match => match[0].length));
  const fence = '`'.repeat(longest + 1);
  return `${fence}text\n${text}\n${fence}`;
}

/** Writes and deletes files in the tree and commits exactly those paths with `--only`, so whatever peers staged stays
 * staged and out of the commit. Empty directories left behind are removed. */
export function commitPaths(repo: string, write: readonly { path: string; content: string | Buffer }[], remove: readonly string[],
  message: string): string | undefined {
  for (const file of write) {
    mkdirSync(dirname(join(repo, file.path)), { recursive: true });
    writeFileSync(join(repo, file.path), file.content);
  }
  const tracked = remove.filter(path => git(repo, ['ls-files', '--', path], true) !== '');
  for (const path of remove) {
    rmSync(join(repo, path), { force: true, recursive: true });
    for (let dir = dirname(path); dir !== '.' && dir !== '' && existsSync(join(repo, dir)) && !readdirSync(join(repo, dir)).length; dir = dirname(dir)) {
      rmSync(join(repo, dir), { recursive: true });
    }
  }
  const paths = [...write.map(file => file.path), ...tracked];
  if (!paths.length) return undefined;
  if (write.length) git(repo, ['add', '--', ...write.map(file => file.path)]);
  git(repo, ['commit', '-q', '--only', '-m', message, '--', ...paths]);
  return git(repo, ['rev-parse', 'HEAD']);
}

/** `file:line:text` of tracked files that mention any of `needles` (fixed strings), outside `exclude` globs
 * (the history-exempt paths by default). */
export function treeMentions(repo: string, needles: readonly string[],
  options: { words?: boolean; exclude?: readonly string[] } = {}): string[] {
  if (!needles.length) return [];
  const excludes = (options.exclude ?? HISTORY_EXEMPT).map(pattern => `:(exclude,glob)${pattern}`);
  const found = git(repo, ['grep', '-n', '-I', '-F', ...options.words ? ['-w'] : [],
    ...needles.flatMap(needle => ['-e', needle]), '--', '.', ...excludes], true);
  return found.split(/\r?\n/).filter(Boolean);
}

export interface DocCommit { hash: string; subject: string; files: string[] }

/** Commits between two revisions that change Markdown and carry no `Goal:` trailer: the maintainer's documentation
 * edits, which a manager re-reads before it briefs again. Managers, goalctl and workers all add the trailer. */
export function maintainerDocCommits(repo: string, since: string, until = 'main'): DocCommit[] {
  const log = gitRaw(repo, ['log', '--no-merges', '--format=%x1e%h%x00%s%x00%(trailers:key=Goal,valueonly,separator=%x2C)%x00',
    '--name-only', `${since}..${until}`, '--', '*.md'], true);
  return log.split('\x1e').filter(Boolean).flatMap(record => {
    const [hash = '', subject = '', goals = '', rest = ''] = record.split('\0');
    return goals.trim() ? [] : [{ hash, subject, files: rest.split(/\r?\n/).filter(Boolean) }];
  });
}

/** Worker-facing documents with uncommitted edits in the main checkout: workers branch from committed `main`, so they
 * would not see them. */
function uncommittedWorkerDocs(goal: string): string[] {
  const docs = ['AGENTS.md', 'GOAL.md', `${GOALS_DIR}/README.md`, `${GOALS_DIR}/worker.md`, goalFile(goal)];
  return parsePorcelainZ(gitRaw(repoRoot(), ['status', '--porcelain', '-z', '--', ...docs], true));
}

// ---------------------------------------------------------------------------------------------------------------
// Processes. Windows has no process groups to signal and reuses PIDs, so liveness compares the process's start time
// with the one recorded at spawn, and termination kills the process tree with taskkill.

interface Kernel32 {
  OpenProcess(access: number, inherit: number, pid: number): unknown;
  GetExitCodeProcess(handle: unknown, code: unknown): number;
  GetProcessTimes(handle: unknown, created: unknown, exited: unknown, kernel: unknown, user: unknown): number;
  CloseHandle(handle: unknown): number;
}
let kernel32: Kernel32 | null | undefined;

function kernel(): Kernel32 | null {
  if (kernel32 !== undefined) return kernel32;
  if (process.platform !== 'win32') return kernel32 = null;
  try {
    kernel32 = dlopen('kernel32.dll', {
      OpenProcess: { args: [FFIType.u32, FFIType.i32, FFIType.u32], returns: FFIType.ptr },
      GetExitCodeProcess: { args: [FFIType.ptr, FFIType.ptr], returns: FFIType.i32 },
      GetProcessTimes: { args: [FFIType.ptr, FFIType.ptr, FFIType.ptr, FFIType.ptr, FFIType.ptr], returns: FFIType.i32 },
      CloseHandle: { args: [FFIType.ptr], returns: FFIType.i32 },
    }).symbols as unknown as Kernel32;
  } catch {
    kernel32 = null;
  }
  return kernel32;
}

const PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;
const STILL_ACTIVE = 259;

export interface ProcessInfo { alive: boolean; mark?: number }

/** Whether a process runs, and its start mark: creation time in ms on Windows, start ticks from /proc on Linux. */
export function processInfo(pid: number): ProcessInfo {
  if (!Number.isInteger(pid) || pid <= 0) return { alive: false };
  const k = kernel();
  if (k) {
    const handle = k.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if (handle) {
      try {
        const code = new Uint32Array(1);
        if (k.GetExitCodeProcess(handle, ptr(code)) && code[0] !== STILL_ACTIVE) return { alive: false };
        const times = new BigUint64Array(4);
        const ok = k.GetProcessTimes(handle, ptr(times, 0), ptr(times, 8), ptr(times, 16), ptr(times, 24));
        return { alive: true, mark: ok ? Number(times[0]! / 10_000n - 11_644_473_600_000n) : undefined };
      } finally {
        k.CloseHandle(handle);
      }
    }
  }
  try {
    process.kill(pid, 0);
  } catch (error) {
    return { alive: (error as NodeJS.ErrnoException).code === 'EPERM' };
  }
  try {
    const stat = readFileSync(`/proc/${pid}/stat`, 'utf8');
    return { alive: true, mark: Number(stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19]) };
  } catch {
    return { alive: true };
  }
}

/** The process with this PID is the one whose start mark was recorded (or no mark could be read). */
export function sameProcess(pid: number, mark?: number): boolean {
  const info = processInfo(pid);
  return info.alive && (mark === undefined || info.mark === undefined || info.mark === mark);
}

/** Terminates a process and its descendants. */
export function killTree(pid: number): void {
  if (process.platform === 'win32') {
    spawnSync('taskkill', ['/PID', String(pid), '/T', '/F'], { windowsHide: true, encoding: 'utf8' });
    return;
  }
  try { process.kill(-pid, 'SIGTERM'); } catch { try { process.kill(pid, 'SIGTERM'); } catch { /* gone */ } }
}

/** Kills the tree and waits until the process is gone; true when it is. */
export async function terminate(pid: number, mark?: number, timeoutMs = 30_000): Promise<boolean> {
  if (!sameProcess(pid, mark)) return true;
  killTree(pid);
  const deadline = Date.now() + timeoutMs;
  while (sameProcess(pid, mark) && Date.now() < deadline) {
    await Bun.sleep(250);
    if (process.platform !== 'win32' && Date.now() > deadline - timeoutMs / 2) {
      try { process.kill(-pid, 'SIGKILL'); } catch { /* gone */ }
    }
  }
  return !sameProcess(pid, mark);
}

const escapeRegex = (text: string): string => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** A case-insensitive pattern for a worktree path in any spelling a command line may carry: `D:\…`, `D:/…` and Git
 * Bash's `/d/…`, followed by a separator, quote, space or the end, so `g-100` never matches `g-1000`. */
export function worktreeProcessPattern(worktree: string): string {
  const native = resolve(worktree);
  const forms = new Set([native, native.replaceAll('\\', '/')]);
  const drive = /^([A-Za-z]):[\\/](.*)$/.exec(native);
  if (drive) forms.add(`/${drive[1]!.toLowerCase()}/${drive[2]!.replaceAll('\\', '/')}`);
  return `(?:${[...forms].map(escapeRegex).join('|')})(?:[\\\\/"'\\s]|$)`;
}

const powershell = (): string => (Bun.which('pwsh') ? 'pwsh' : 'powershell');

/** Worker-started servers, browsers and test binaries can outlive the worker's tree (taskkill /T misses descendants
 * whose parent already exited) and hold files that block removing the worktree. Kills every process whose executable
 * or command line lies in the worktree. Returns the PIDs it killed. */
export function killWorktreeProcesses(worktree: string): number[] {
  const skip = new Set([process.pid, process.ppid]);
  let victims: number[] = [];
  if (process.platform === 'win32') {
    const script = '$p = $env:GOAL_SWEEP_PATTERN; Get-CimInstance Win32_Process | Where-Object {'
      + ' ($_.CommandLine -and $_.CommandLine -match $p) -or ($_.ExecutablePath -and $_.ExecutablePath -match $p)'
      + ' } | ForEach-Object { $_.ProcessId }';
    const listed = spawnSync(powershell(), ['-NoProfile', '-NonInteractive', '-Command', script], { encoding: 'utf8',
      windowsHide: true, timeout: 60_000, env: { ...process.env, GOAL_SWEEP_PATTERN: worktreeProcessPattern(worktree) } });
    victims = (listed.stdout ?? '').split(/\r?\n/).map(Number).filter(pid => Number.isInteger(pid) && pid > 0 && !skip.has(pid));
    for (const pid of victims) killTree(pid);
    return victims;
  }
  if (!existsSync('/proc')) return [];
  const prefix = worktree.endsWith('/') ? worktree : `${worktree}/`;
  for (const entry of readdirSync('/proc')) {
    const pid = Number(entry);
    if (!Number.isInteger(pid) || skip.has(pid)) continue;
    try {
      const cwd = readlinkSync(`/proc/${pid}/cwd`);
      if (cwd === worktree || cwd.startsWith(prefix)) victims.push(pid);
    } catch { /* exited or not ours */ }
  }
  for (const pid of victims) { try { process.kill(pid, 'SIGTERM'); } catch { /* gone */ } }
  return victims;
}

// ---------------------------------------------------------------------------------------------------------------
// Locks. A lock is a directory renamed into place with its owner record already inside, so a holder is never
// half-written. A holder that crashed leaves the directory behind; it counts as free once its process is gone or its
// PID belongs to a newer process.

export interface LockOwner { pid: number; mark?: number; token: string; acquiredAt: string; goal?: string; command?: string }

function readOwner(path: string): LockOwner | undefined {
  try { return JSON.parse(readFileSync(join(path, 'owner.json'), 'utf8')) as LockOwner; } catch { return undefined; }
}

/** The live holder of a lock, or undefined when it is free or its holder is gone. */
export function lockHolder(path: string): LockOwner | undefined {
  const owner = readOwner(path);
  return owner && sameProcess(owner.pid, owner.mark) ? owner : undefined;
}

function discard(path: string): void {
  const aside = `${path}.stale-${randomUUID()}`;
  try { renameSync(path, aside); } catch { return; }
  rmSync(aside, { recursive: true, force: true });
}

/** Takes the lock if it is free or stale; returns its release function, or undefined while another process holds it. */
export function tryLock(path: string, info: { goal?: string; command?: string } = {}): (() => void) | undefined {
  const owner: LockOwner = { pid: process.pid, mark: processInfo(process.pid).mark, token: randomUUID(),
    acquiredAt: new Date().toISOString(), ...info };
  const staging = `${path}.${owner.token}.tmp`;
  mkdirSync(staging, { recursive: true });
  writeFileSync(join(staging, 'owner.json'), JSON.stringify(owner));
  try {
    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        renameSync(staging, path);
        return () => { if (readOwner(path)?.token === owner.token) discard(path); };
      } catch {
        const current = readOwner(path);
        if (current && sameProcess(current.pid, current.mark)) return undefined;
        // A directory without an owner record is one a crashed release left half-deleted, or one being discarded.
        if (!current && existsSync(path) && Date.now() - statSync(path).mtimeMs < 10_000) return undefined;
        if (readOwner(path)?.token === current?.token) discard(path);
      }
    }
    return undefined;
  } finally {
    rmSync(staging, { recursive: true, force: true });
  }
}

export async function acquireLock(path: string, options: { timeoutMs: number; pollMs: number; label: string;
  info?: { goal?: string; command?: string } }): Promise<() => void> {
  mkdirSync(dirname(path), { recursive: true });
  const deadline = Date.now() + options.timeoutMs;
  let announced = false;
  for (;;) {
    const release = tryLock(path, options.info);
    if (release) return release;
    if (Date.now() > deadline) throw new Error(`Timed out waiting for ${options.label}`);
    if (!announced && options.timeoutMs > 60_000) {
      const holder = lockHolder(path);
      console.error(`waiting for ${options.label}${holder ? ` held by pid ${holder.pid}${holder.command ? `: ${holder.command}` : ''}` : ''}`);
      announced = true;
    }
    await Bun.sleep(options.pollMs);
  }
}

function readLedger(): Ledger {
  return existsSync(ledgerPath()) ? JSON.parse(readFileSync(ledgerPath(), 'utf8')) as Ledger : { tasks: {} };
}

function writeLedger(ledger: Ledger): void {
  writeFileSync(`${ledgerPath()}.tmp`, `${JSON.stringify(ledger, null, 2)}\n`);
  renameSync(`${ledgerPath()}.tmp`, ledgerPath());
}

/** Runs a change under the ledger lock and writes the ledger afterwards, also when the change returns a refusal. */
async function withLedger<T>(change: (ledger: Ledger) => T | Promise<T>): Promise<T> {
  mkdirSync(stateDir(), { recursive: true });
  const release = await acquireLock(join(stateDir(), 'ledger.lock'), { timeoutMs: 120_000, pollMs: 100, label: 'ledger lock' });
  try {
    const ledger = readLedger();
    const result = await change(ledger);
    writeLedger(ledger);
    return result;
  } finally {
    release();
  }
}

// ---------------------------------------------------------------------------------------------------------------
// Ledger helpers.

const activeGoals = (ledger: Ledger): string[] =>
  Object.entries(ledger.goals ?? {}).filter(([, goal]) => !goal.closedAt).map(([slug]) => slug);

const managerOf = (ledger: Ledger, goal: string): string => ledger.goals?.[goal]?.manager || 'goal-manager';

/** Areas are read from each Goal file at every check, so the maintainer's edits apply at once. */
function areasOf(slugs: readonly string[]): Record<string, string[]> {
  return Object.fromEntries(slugs.map(slug => {
    const path = join(repoRoot(), goalFile(slug));
    return [slug, existsSync(path) ? goalAreas(readFileSync(path, 'utf8')) : []];
  }));
}

function assertOwner(task: Task): void {
  const refusal = ownerRefusal(task);
  if (refusal) throw new Error(refusal);
}

function taskOf(ledger: Ledger, id: string): Task {
  const task = ledger.tasks[id.toUpperCase()];
  if (!task) throw new Error(`Unknown task ${id || '(missing)'}`);
  return task;
}

function lastAttempt(task: Task): Attempt {
  const attempt = task.attempts.at(-1);
  if (!attempt) throw new Error(`${task.id} has no attempt`);
  return attempt;
}

const attemptAlive = (attempt: Attempt): boolean => sameProcess(attempt.pid, attempt.pidMark);
const running = (task: Task): boolean => task.state === 'running' && attemptAlive(lastAttempt(task));
const isOpen = (task: Task): boolean => !CLOSED.includes(task.state);

/** Open tasks that work in the same tree, the task itself included. */
function sharersOf(ledger: Ledger, task: Task): Task[] {
  return Object.values(ledger.tasks).filter(other => other.worktree === task.worktree && isOpen(other));
}

/** A date as YYYY-MM-DD in the host's time zone, the one the maintainer's dates use. */
export function localDate(date: Date): string {
  const pad = (value: number): string => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** A Goal's directory under archive/goals, named after its slug and start date. */
function archiveDirOf(ledger: Ledger, goal: string): string {
  const started = ledger.goals?.[goal]?.startedAt;
  return ledger.goals?.[goal]?.archive ?? `${ARCHIVE_DIR}/${goal}-${localDate(started ? new Date(started) : new Date())}`;
}

function elapsed(fromIso: string): string {
  const minutes = Math.round((Date.now() - Date.parse(fromIso)) / 60_000);
  return `${Math.floor(minutes / 60)}h${String(minutes % 60).padStart(2, '0')}m`;
}

// ---------------------------------------------------------------------------------------------------------------
// Worker results.

export interface AttemptResult { text: string; session?: string; error: boolean; cost?: number; tokens?: string; usage?: TokenUse }
export interface TokenUse { input: number; cached: number; output: number }

const errorTail = (attempt: Attempt): string => {
  const errPath = attempt.output.replace(/\.json$/, '.err');
  return existsSync(errPath) ? readFileSync(errPath, 'utf8').slice(-3000) : '';
};

function readCodexResult(attempt: Attempt): AttemptResult {
  const lines = existsSync(attempt.output) ? readFileSync(attempt.output, 'utf8').split(/\r?\n/) : [];
  let session: string | undefined;
  const usage: TokenUse = { input: 0, cached: 0, output: 0 };
  let failure = '';
  for (const line of lines) {
    if (!line.startsWith('{')) continue;
    try {
      const event = JSON.parse(line) as { type?: string; thread_id?: string; message?: string; error?: { message?: string };
        usage?: { input_tokens?: number; cached_input_tokens?: number; output_tokens?: number } };
      if (event.type === 'thread.started') session ??= event.thread_id;
      if (event.type === 'turn.completed') {
        usage.input += event.usage?.input_tokens ?? 0;
        usage.cached += event.usage?.cached_input_tokens ?? 0;
        usage.output += event.usage?.output_tokens ?? 0;
      }
      if (event.type === 'error' || event.type === 'turn.failed') failure = event.message ?? event.error?.message ?? failure;
    } catch { /* partial line */ }
  }
  const text = attempt.lastMessage && existsSync(attempt.lastMessage) ? readFileSync(attempt.lastMessage, 'utf8') : '';
  return { text: text || `(no final message)\n${failure}\n${errorTail(attempt)}`.trim(), session, error: !text, usage,
    tokens: `input ${usage.input} (cached ${usage.cached}), output ${usage.output}` };
}

/** The final message, session and token use of an attempt. Claude Code and the Grok CLI print one JSON result. */
export function readResult(attempt: Attempt): AttemptResult {
  if (engineOf(attempt) === 'codex') return readCodexResult(attempt);
  const raw = existsSync(attempt.output) ? readFileSync(attempt.output, 'utf8') : '';
  const candidates = [raw, ...raw.split(/\r?\n/).filter(line => line.startsWith('{')).reverse()];
  for (const candidate of candidates) {
    try {
      const data = JSON.parse(candidate) as Record<string, unknown>;
      const text = String(data.result ?? data.text ?? '');
      const use = (data.usage ?? {}) as Record<string, number | undefined>;
      const usage: TokenUse = { input: (use.input_tokens ?? 0) + (use.cache_creation_input_tokens ?? 0),
        cached: use.cache_read_input_tokens ?? 0, output: use.output_tokens ?? 0 };
      return { text: data.is_error === true ? `${text}\n${errorTail(attempt)}`.trim() : text,
        session: (data.session_id ?? data.sessionId) as string | undefined, error: data.is_error === true || !text,
        cost: data.total_cost_usd as number | undefined, usage,
        tokens: `input ${usage.input} (cache read ${usage.cached}), output ${usage.output}` };
    } catch { /* try the next candidate */ }
  }
  return { text: `(no parsable output)\n${errorTail(attempt)}`.trim(), error: true };
}

/** Every attempt's final message. Each sits in a fence: worker output is plain text whose links were never meant to
 * resolve from the archive. */
function handoffText(task: Task): string {
  return [`# ${task.id} handoffs: ${task.title}`, '',
    ...task.attempts.flatMap(attempt => [`## Attempt ${attempt.n} (${modelOf(engineOf(attempt))}/${attempt.effort}, `
      + `${attempt.startedAt})`, '', fenced(readResult(attempt).text.trim()), ''])].join('\n');
}

// Codex records the account's rate limits with each turn in its session rollouts, so the newest rollout line that
// carries them is the account's current usage.
export interface CodexUsage { used?: number; windowMinutes?: number; resetInHours?: number; plan?: string; reached?: boolean;
  ageSeconds?: number }

export function parseCodexUsage(text: string, nowMs: number): CodexUsage {
  const now = nowMs / 1000;
  const lines = text.split(/\r?\n/);
  for (let i = lines.length - 1; i >= 0; i--) {
    if (!lines[i]!.includes('"rate_limits"')) continue;
    try {
      type Window = { used_percent?: number; window_minutes?: number; resets_at?: number };
      const event = JSON.parse(lines[i]!) as { timestamp?: string; payload?: { rate_limits?: {
        primary?: Window | null; secondary?: Window | null; plan_type?: string; rate_limit_reached_type?: string | null } } };
      const limits = event.payload?.rate_limits;
      const windows = [limits?.primary, limits?.secondary].filter((w): w is Window => typeof w?.used_percent === 'number');
      if (!limits || !windows.length) continue;
      const at = event.timestamp ? Date.parse(event.timestamp) / 1000 : undefined;
      const ageSeconds = at === undefined ? undefined : Math.round(now - at);
      // A window whose reset has passed starts empty again.
      const live = windows.filter(w => (w.resets_at ?? Infinity) > now);
      if (!live.length) return { used: 0, reached: false, plan: limits.plan_type, ageSeconds };
      const fullest = live.reduce((a, b) => (b.used_percent! > a.used_percent! ? b : a));
      return { used: fullest.used_percent, windowMinutes: fullest.window_minutes, plan: limits.plan_type, ageSeconds,
        resetInHours: fullest.resets_at === undefined ? undefined : Math.round((fullest.resets_at - now) / 360) / 10,
        reached: fullest.used_percent! >= 100 || !!limits.rate_limit_reached_type };
    } catch { /* partial line */ }
  }
  return {};
}

function tail(file: string, bytes: number): string {
  const fd = openSync(file, 'r');
  try {
    const size = statSync(file).size;
    const buffer = Buffer.alloc(Math.min(bytes, size));
    readSync(fd, buffer, 0, buffer.length, size - buffer.length);
    return buffer.toString('utf8');
  } finally {
    closeSync(fd);
  }
}

/** The Codex account's usage from its newest rollouts (sessions/YYYY/MM/DD/*.jsonl of the last three session days). */
function codexUsage(): CodexUsage {
  const children = (dir: string): string[] => {
    try { return readdirSync(dir).sort().reverse().map(name => join(dir, name)); } catch { return []; }
  };
  const days = children(join(codexHome(), 'sessions')).flatMap(children).flatMap(children).slice(0, 3);
  const files = days.flatMap(day => children(day).filter(file => file.endsWith('.jsonl')))
    .map(file => ({ file, at: statSync(file).mtimeMs })).sort((a, b) => b.at - a.at);
  for (const { file } of files) {
    const usage = parseCodexUsage(tail(file, 512 * 1024), Date.now());
    if (usage.used !== undefined) return usage;
  }
  return {};
}

function describeCodex(usage: CodexUsage): string {
  return `codex (${modelOf('codex')}): ` + (usage.used === undefined ? 'no usage recorded yet'
    : `${usage.used}% of ${usage.windowMinutes ? `${Math.round(usage.windowMinutes / 1440)}d` : 'window'}`
      + `${usage.reached ? ' EXHAUSTED' : ''}`
      + (usage.resetInHours !== undefined ? `, resets in ${usage.resetInHours}h` : '')
      + (usage.ageSeconds !== undefined ? ` (${Math.round(usage.ageSeconds / 60)}m old)` : ''));
}

// ---------------------------------------------------------------------------------------------------------------
// Commands.

function launch(task: Task, effort: string, session: string, prompt: string, resume: boolean,
  manager: string, engine: Engine = engineOf(task)): Attempt {
  const n = task.attempts.length + 1;
  const runDir = join(stateDir(), 'runs', task.id);
  mkdirSync(runDir, { recursive: true });
  const output = join(runDir, `attempt-${n}.json`);
  const promptFile = join(runDir, `attempt-${n}.prompt.md`);
  writeFileSync(promptFile, prompt);
  const lastMessage = engine === 'codex' ? join(runDir, `attempt-${n}.last.md`) : undefined;
  const [program, args] = launchCommand({ id: task.id, effort, session, prompt, resume, engine,
    worktree: task.worktree, lastMessage });
  const spec: LaunchSpec = { program, args, cwd: task.worktree, stdin: promptOnStdin(engine) ? promptFile : devNull,
    stdout: output, stderr: join(runDir, `attempt-${n}.err`), exit: join(runDir, `attempt-${n}.exit`) };
  const specFile = join(runDir, `attempt-${n}.launch.json`);
  writeFileSync(specFile, JSON.stringify(spec, null, 2));
  // The launcher is detached so it survives the manager and its shell, and leaves the spawner's job object; see
  // `runWorker` for why the worker itself is not.
  const child = spawn(process.execPath, [import.meta.path, '__run', specFile], { cwd: task.worktree, detached: true,
    windowsHide: true, stdio: 'ignore',
    env: workerEnv(process.env, { ...engine === 'codex' && process.env.GOAL_CODEX_HOME ? { CODEX_HOME: codexHome() } : {},
      ...cargoEnv(), GOAL_TASK_ID: task.id, GOAL_ID: task.goal, GOAL_MANAGER: manager }) });
  child.on('error', () => { /* reported through the missing PID below */ });
  child.unref();
  if (!child.pid) throw new Error(`Could not start the worker launcher for ${program}`);
  return { n, effort, engine, pid: child.pid, pidMark: processInfo(child.pid).mark, session, output, lastMessage,
    startedAt: new Date().toISOString() };
}

export interface LaunchSpec { program: string; args: string[]; cwd: string; stdin: string; stdout: string; stderr: string; exit: string }

/** The worker's parent: `bun goalctl.ts __run <spec>`. A detached process has no console on Windows, and every console
 * program it starts without CREATE_NO_WINDOW opens a console window of its own; git started that way also hung or
 * failed with no output. So the launcher runs detached and starts the worker with a hidden console that every tool the
 * worker runs inherits. It waits for the worker, records its exit code and exits with it; its PID is the attempt's, so
 * stopping it kills the worker's whole tree. */
export async function runWorker(specFile: string): Promise<number> {
  const spec = JSON.parse(readFileSync(specFile, 'utf8')) as LaunchSpec;
  const fds = [openSync(spec.stdin, 'r'), openSync(spec.stdout, 'w'), openSync(spec.stderr, 'w')] as const;
  const code = await new Promise<number>(done => {
    const child = spawn(spec.program, spec.args, { cwd: spec.cwd, stdio: [...fds], windowsHide: true });
    child.on('error', error => {
      writeFileSync(fds[2], `goalctl could not start ${spec.program}: ${error.message}\n`);
      done(127);
    });
    child.on('exit', code => done(code ?? 1));
  });
  for (const fd of fds) closeSync(fd);
  writeFileSync(spec.exit, String(code));
  return code;
}

function changedFiles(task: Task): { committed: string[]; dirty: string[]; ahead: number } {
  if (!existsSync(task.worktree)) return { committed: [], dirty: [], ahead: 0 };
  const root = repoRoot();
  const committed = nulList(gitRaw(root, ['diff', '--name-only', '-z', `main...${task.branch}`], true));
  const dirty = parsePorcelainZ(gitRaw(task.worktree, ['status', '--porcelain', '-z', '--untracked-files=all'], true));
  const ahead = Number(git(root, ['rev-list', '--count', `main..${task.branch}`], true) || 0);
  return { committed, dirty, ahead };
}

/** The branch's changes since it left main, with each file's content on both sides for `historyIntroductions`. */
function branchChanges(task: Task): ChangedFile[] {
  const root = repoRoot();
  const base = git(root, ['merge-base', 'main', task.branch]);
  const show = (revision: string, path: string): string => gitRaw(root, ['show', `${revision}:${path}`], true);
  const entries = gitRaw(root, ['diff', '--name-status', '-z', '-M', `${base}..${task.branch}`]).split('\0');
  const files: ChangedFile[] = [];
  for (let i = 0; i < entries.length; i++) {
    const code = entries[i]!;
    if (!code) continue;
    const kind = code[0]!;
    const status = (['A', 'D', 'R'].includes(kind) ? kind : 'M') as ChangedFile['status'];
    const from = kind === 'R' || kind === 'C' ? entries[++i] : undefined;
    const path = entries[++i] ?? '';
    files.push({ path, status, ...status === 'R' ? { from } : {},
      before: status === 'A' ? undefined : show(base, from ?? path), after: status === 'D' ? undefined : show(task.branch, path) });
  }
  return files;
}

function describe(task: Task, paths = task.paths): string {
  const { committed, dirty, ahead } = changedFiles(task);
  const violations = outOfScope([...new Set([...committed, ...dirty])], paths);
  return [
    `${task.branch}: ${ahead} commit(s) ahead of main; worktree ${dirty.length ? `DIRTY (${dirty.length} files)` : 'clean'}`,
    violations.length ? `scope: VIOLATIONS\n  ${violations.join('\n  ')}` : 'scope: ok',
  ].join('\n');
}

/** Resolves a brief argument to its repository path and checks that it sits in its Goal's tasks directory under its
 * own ID. */
function briefLocation(briefPath: string, brief: Brief): { absolute: string; path: string; goal: string } {
  // Git expands Windows short names; resolve both sides so aliases cannot make an in-repository brief look external.
  const absolute = realpathSync.native(resolve(briefPath));
  const path = repoPath(absolute);
  const goal = goalOfBriefPath(path);
  if (!goal) throw new Error(`${path}: briefs live at ${GOALS_DIR}/<goal>/tasks/G-NNN.md (\`new\` writes them there)`);
  if (basename(path, '.md') !== brief.id) throw new Error(`${path} declares id ${brief.id}; the file name and id must match`);
  const caller = process.env.GOAL_ID;
  if (caller && caller !== goal) throw new Error(`${path} belongs to Goal ${goal}, not ${caller}`);
  return { absolute, path, goal };
}

function readValidBrief(briefPath: string): Brief {
  const brief = parseBrief(readFileSync(resolve(briefPath), 'utf8'));
  const errors = validateBrief(brief);
  if (errors.length) throw new Error(`Invalid brief ${briefPath}:\n  ${errors.join('\n  ')}`);
  return brief;
}

const maxWorkers = (): number => Number(process.env.GOAL_MAX_WORKERS || 3);

async function dispatch(briefPath: string, flags: Set<string>): Promise<void> {
  if (!briefPath) throw new Error('dispatch needs a brief path');
  const brief = readValidBrief(briefPath);
  const { absolute, path, goal } = briefLocation(briefPath, brief);
  const root = repoRoot();
  // Outside the ledger lock: a probe may take seconds.
  if (!flags.has('--skip-preflight')) preflight(brief.engine, root);
  await withLedger(ledger => {
    if (ledger.tasks[brief.id]) throw new Error(`${brief.id} already exists (${ledger.tasks[brief.id]!.state}); use resume`);
    const record = ledger.goals?.[goal];
    if (!record || record.closedAt) throw new Error(`Goal ${goal} is not started; run \`goal start ${goal} --manager <session>\``);
    const reservedBy = ledger.reserved?.[brief.id];
    if (reservedBy && reservedBy !== goal) throw new Error(`${brief.id} is reserved for Goal ${reservedBy}; take an ID with \`new\``);
    const tasks = Object.values(ledger.tasks);
    const conflicts = claimConflicts(brief, tasks);
    if (!flags.has('--allow-area')) conflicts.push(...areaConflicts(brief.paths, goal, areasOf(activeGoals(ledger))));
    if (conflicts.length) throw new Error(`Claim conflict for ${brief.id}:\n  ${conflicts.join('\n  ')}`);
    for (const id of brief.depends) {
      const dependency = ledger.tasks[id];
      if (dependency && !['merged', 'verified'].includes(dependency.state)) {
        throw new Error(`${brief.id} depends on ${id}, which is ${dependency.state}`);
      }
    }
    const live = tasks.filter(running).length;
    if (live >= maxWorkers()) throw new Error(`Concurrency limit reached: ${live}/${maxWorkers()} live workers`);
    const engine = brief.engine;
    const codex = engine === 'codex' ? codexUsage() : undefined;
    if (codex?.reached && !flags.has('--force-usage')) throw new Error(`${describeCodex(codex)}; use another engine or pass --force-usage`);
    // The worker reads the documents committed on main; tell the manager what it has not adopted or re-read.
    const unadopted = uncommittedWorkerDocs(goal);
    if (unadopted.length) {
      console.error(`warning: uncommitted edits the worker will not see: ${unadopted.join(', ')}; commit stable maintainer`
        + ' edits ("Adopt maintainer documentation update") before dispatching work that depends on them');
    }
    const head = git(root, ['rev-parse', 'main']);
    const edits = record.seenHead ? maintainerDocCommits(root, record.seenHead, head) : [];
    if (edits.length) {
      console.log(`Documentation changed on main without a Goal trailer since this Goal's last dispatch; re-read it:\n  `
        + edits.map(commit => `${commit.hash} ${commit.subject}: ${commit.files.join(', ')}`).join('\n  '));
    }
    if (flags.has('--dry-run')) {
      console.log(`${brief.id}: claims ok; ${live}/${maxWorkers()} live; ${modelOf(engine)}/${brief.effort}`
        + `${codex ? `; ${describeCodex(codex)}` : ''}`);
      return;
    }
    const name = brief.worktree ?? brief.id.toLowerCase();
    const worktree = join(root, '.temp', 'worktrees', name);
    const branch = `goal/${name}`;
    const reuse = brief.worktree !== undefined && existsSync(worktree);
    if (existsSync(worktree) && !reuse) throw new Error(`${worktree} already exists; remove it or use another ID`);
    if (!reuse) {
      // A shared branch outlives its worktree once its last task closes; reattach to it.
      const branchExists = git(root, ['rev-parse', '--verify', '--quiet', `refs/heads/${branch}`], true) !== '';
      git(root, branchExists && brief.worktree !== undefined ? ['worktree', 'add', '-q', worktree, branch]
        : ['worktree', 'add', '-q', worktree, '-b', branch, 'main']);
    }
    mkdirSync(join(worktree, '.temp', 'goal'), { recursive: true });
    const { worktree: _shared, migrations: _none, ...claims } = brief;
    const task: Task = { ...claims, brief: path, worktree, branch,
      base: reuse ? git(worktree, ['merge-base', 'HEAD', 'main']) : head,
      state: 'running', attempts: [], ...brief.worktree ? { worktreeName: brief.worktree } : {}, goal };
    copyFileSync(absolute, join(worktree, briefFile({ ...task, shared: !!task.worktreeName })));
    const manager = managerOf(ledger, goal);
    task.attempts.push(launch(task, brief.effort, isClaudeCode(engine) ? randomUUID() : '',
      workerPrompt(task, manager, engine), false, manager, engine));
    ledger.tasks[brief.id] = task;
    if (ledger.reserved) delete ledger.reserved[brief.id];
    ledger.lastId = Math.max(ledger.lastId ?? 0, Number(brief.id.slice(2)));
    ledger.startedAt ??= new Date().toISOString();
    record.seenHead = head;
    console.log(`${brief.id} started: ${modelOf(engine)}/${brief.effort} pid ${lastAttempt(task).pid} in ${worktree}`);
    console.log(`Next: run \`task goal -- wait ${brief.id}\` in the background.`);
  });
}

async function waitFor(id: string): Promise<void> {
  // A killed waiter must never record the worker as ended (an expired monitor once marked live REZICS workers exited).
  for (const signal of ['SIGTERM', 'SIGINT', 'SIGHUP'] as const) {
    try { process.on(signal, () => process.exit(143)); } catch { /* not available on this platform */ }
  }
  const initial = taskOf(readLedger(), id);
  const attempt = lastAttempt(initial);
  const poll = Number(process.env.GOAL_POLL_MS || 5000);
  const alive = (): boolean => attemptAlive(attempt);
  while (alive() || (await Bun.sleep(Math.min(poll, 3000)), alive())) await Bun.sleep(poll);
  const result = readResult(attempt);
  const task = await withLedger(ledger => {
    const current = taskOf(ledger, id);
    const last = lastAttempt(current);
    if (last.n === attempt.n) {
      last.endedAt ??= new Date().toISOString();
      if (result.session && !last.session) last.session = result.session;
      if (current.state === 'running') current.state = 'exited';
    }
    return current;
  });
  const handoffs = join(stateDir(), 'handoffs');
  mkdirSync(handoffs, { recursive: true });
  writeFileSync(join(handoffs, `${task.id}.md`), handoffText(task));
  const exitFile = attempt.output.replace(/\.json$/, '.exit');
  const exit = existsSync(exitFile) ? `; exit code ${readFileSync(exitFile, 'utf8').trim()}` : '';
  console.log(`${task.id} attempt ${attempt.n} ended after ${elapsed(attempt.startedAt)}${exit}`
    + `${result.error ? ' WITH ERROR' : ''}${result.cost !== undefined ? `; cost $${result.cost.toFixed(2)}` : ''}`
    + `${result.tokens ? `; ${modelOf(engineOf(attempt))} tokens ${result.tokens}` : ''}`);
  const hint = result.error ? failureHint(engineOf(attempt), result.text) : undefined;
  if (hint) console.log(hint);
  const ledger = readLedger();
  console.log(describe(task, sharersOf(ledger, task).flatMap(other => other.paths)));
  console.log(`--- handoff (also in .temp/goal/handoffs/${task.id}.md) ---\n`
    + `${result.text.length > 8000 ? `${result.text.slice(0, 8000)}\n[truncated]` : result.text}`);
}

async function resumeTask(id: string, args: string[]): Promise<void> {
  let message = '';
  let effort: string | undefined;
  let engine: Engine | undefined;
  let fresh = false;
  let skipPreflight = false;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '-m') message = args[++i] ?? '';
    else if (args[i] === '--file') message = readFileSync(args[++i] ?? '', 'utf8');
    else if (args[i] === '--effort') effort = args[++i];
    else if (args[i] === '--engine') {
      const value = args[++i];
      if (!isEngine(value)) throw new Error(`--engine must be one of ${ENGINES.join(', ')}`);
      engine = value;
    } else if (args[i] === '--fresh') fresh = true;
    else if (args[i] === '--skip-preflight') skipPreflight = true;
    else throw new Error(`Unsupported resume option: ${args[i]}`);
  }
  if (!message.trim()) throw new Error('resume needs -m <message> or --file <path>');
  if (!skipPreflight) {
    const known = readLedger().tasks[id.toUpperCase()];
    if (known?.attempts.length) preflight(engine ?? engineOf(lastAttempt(known)), repoRoot());
  }
  await withLedger(ledger => {
    const task = taskOf(ledger, id);
    assertOwner(task);
    if (running(task)) throw new Error(`${task.id} is still running; stop it first (the manager never messages a running worker)`);
    if (!isOpen(task)) throw new Error(`${task.id} is closed`);
    if (!existsSync(task.worktree)) throw new Error(`${task.worktree} no longer exists`);
    const previous = lastAttempt(task);
    const nextEngine = engine ?? engineOf(previous);
    const nextEffort = effort ?? previous.effort;
    if (!effortsOf(nextEngine).includes(nextEffort)) {
      throw new Error(`${nextEngine} effort must be one of ${effortsOf(nextEngine).join(', ')}`);
    }
    const manager = managerOf(ledger, task.goal);
    // A session continues only on its own engine: switching back resumes that engine's latest session, and an engine
    // without one starts fresh on the same worktree.
    const sameEngine = [...task.attempts].reverse().find(attempt => engineOf(attempt) === nextEngine
      && (attempt.session || readResult(attempt).session));
    const previousSession = sameEngine ? sameEngine.session || readResult(sameEngine).session || '' : '';
    const continuing = !fresh && !!previousSession;
    const session = continuing ? previousSession : isClaudeCode(nextEngine) ? randomUUID() : '';
    const prompt = continuing ? message : `${workerPrompt(task, manager, nextEngine, nextEffort)}\n\nManager note:\n${message}`;
    task.attempts.push(launch(task, nextEffort, session, prompt, continuing, manager, nextEngine));
    task.engine = nextEngine;
    task.state = 'running';
    console.log(`${task.id} attempt ${task.attempts.length}: ${modelOf(nextEngine)}/${nextEffort}`
      + ` ${continuing ? 'resumed' : 'fresh session'}, pid ${lastAttempt(task).pid}`);
  });
}

/** Handoffs cite files under the worktree's `.temp` (traces, reports, patches); keep them beside the task's run
 * records when the worktree goes. */
export function preserveWorktreeArtifacts(worktree: string, runDir: string): string | null {
  const source = join(worktree, '.temp');
  if (!existsSync(source)) return null;
  mkdirSync(runDir, { recursive: true });
  let target = join(runDir, 'worktree-temp');
  if (existsSync(target)) target = `${target}-${Date.now()}`;
  renameSync(source, target);
  return target;
}

async function stopTask(id: string): Promise<void> {
  const ledger = readLedger();
  const task = taskOf(ledger, id);
  assertOwner(task);
  const attempt = lastAttempt(task);
  if (!await terminate(attempt.pid, attempt.pidMark)) throw new Error(`${task.id} pid ${attempt.pid} did not exit`);
  // In a shared worktree the other workers' processes match the sweep too.
  if (!sharersOf(ledger, task).some(other => other.id !== task.id && running(other))) killWorktreeProcesses(task.worktree);
  await withLedger(current => {
    const stopped = taskOf(current, id);
    lastAttempt(stopped).endedAt ??= new Date().toISOString();
    if (stopped.state === 'running') stopped.state = 'stopped';
  });
  console.log(`${task.id} stopped; its worktree and claims remain until close`);
}

async function mergeTask(id: string, flags: Set<string>): Promise<void> {
  // A refusal that changes state is returned, so withLedger writes `conflict` before the error is thrown.
  const failure = await withLedger((ledger): string | undefined => {
    const root = repoRoot();
    const task = taskOf(ledger, id);
    assertOwner(task);
    const group = sharersOf(ledger, task);
    for (const member of group) if (running(member)) throw new Error(`${member.id} is still running in ${task.worktree}`);
    if (!['exited', 'conflict', 'stopped'].includes(task.state)) throw new Error(`${task.id} is ${task.state}`);
    if (git(root, ['symbolic-ref', '--short', 'HEAD']) !== 'main') throw new Error('Main checkout is not on main');
    const { committed, dirty, ahead } = changedFiles(task);
    if (dirty.length) throw new Error(`${task.id} worktree has uncommitted files:\n  ${dirty.join('\n  ')}`);
    const members = group.filter(member => ['exited', 'conflict', 'stopped'].includes(member.state));
    if (flags.has('--landed')) {
      // The manager already landed this work on main by hand (a cherry-pick, often with a conflict resolved).
      for (const member of members) Object.assign(member, { state: 'merged', mergedCommit: git(root, ['rev-parse', 'HEAD']) });
      console.log(`${members.map(member => member.id).join(', ')} recorded as landed`);
      return undefined;
    }
    if (!ahead) {
      // A resumed task whose earlier commits already landed may hand off with nothing new.
      if (task.mergedCommit && spawnSync('git', ['merge-base', '--is-ancestor', task.branch, 'main'], { cwd: root }).status === 0) {
        task.state = 'merged';
        console.log(`${task.id} has nothing new; its branch is already in main`);
        return undefined;
      }
      throw new Error(`${task.id} has no commits to merge`);
    }
    // Files merged with git's union driver take concurrent appends, so any task may add to them.
    const union = new Set(committed.filter(file => git(root, ['check-attr', 'merge', '--', file], true).endsWith(': merge: union')));
    // A shared worktree's branch carries all of its tasks' work; their claims together are its scope.
    const violations = outOfScope(committed.filter(file => !union.has(file)), group.flatMap(member => member.paths));
    if (violations.length && !flags.has('--allow-scope')) {
      throw new Error(`${task.id} changed files outside its claim:\n  ${violations.join('\n  ')}`);
    }
    const history = flags.has('--allow-ids') ? [] : historyIntroductions(branchChanges(task));
    if (history.length) {
      throw new Error(`${task.id} names tasks in the tree; task IDs belong in commit messages:\n  ${history.join('\n  ')}`);
    }
    const rebase = spawnSync('git', ['rebase', 'main'], { cwd: task.worktree, encoding: 'utf8', windowsHide: true });
    if (rebase.status !== 0) {
      const conflicted = nulList(gitRaw(task.worktree, ['diff', '--name-only', '-z', '--diff-filter=U'], true));
      git(task.worktree, ['rebase', '--abort'], true);
      task.state = 'conflict';
      return `${task.id} does not rebase onto main; it is marked conflict for its worker to resolve:\n  `
        + `${conflicted.join('\n  ') || rebase.stderr.trim()}`;
    }
    const merge = spawnSync('git', ['merge', '--ff-only', '-q', task.branch], { cwd: root, encoding: 'utf8', windowsHide: true });
    if (merge.status !== 0) throw new Error(`Fast-forward failed in the main checkout:\n${merge.stderr}`);
    const head = git(root, ['rev-parse', 'HEAD']);
    for (const member of members) Object.assign(member, { state: 'merged', mergedCommit: head });
    console.log(`${members.map(member => member.id).join(', ')} merged at ${head.slice(0, 12)}; ${committed.length} file(s):`);
    console.log(`  ${committed.join('\n  ')}`);
    return undefined;
  });
  if (failure) throw new Error(failure);
}

// Re-reads an updated brief for an open task and replaces its claims after the same conflict checks as dispatch; the
// new brief is copied into the worktree.
async function reclaimTask(id: string, briefPath: string): Promise<void> {
  const brief = readValidBrief(briefPath);
  const { absolute, path } = briefLocation(briefPath, brief);
  await withLedger(ledger => {
    const task = taskOf(ledger, id);
    assertOwner(task);
    if (brief.id !== task.id) throw new Error(`${briefPath} is for ${brief.id}, not ${task.id}`);
    if (running(task)) throw new Error(`${task.id} is still running`);
    if (!isOpen(task)) throw new Error(`${task.id} is closed`);
    const conflicts = [...claimConflicts(brief, Object.values(ledger.tasks)),
      ...areaConflicts(brief.paths, task.goal, areasOf(activeGoals(ledger)))];
    if (conflicts.length) throw new Error(`Claim conflict for ${brief.id}:\n  ${conflicts.join('\n  ')}`);
    Object.assign(task, { title: brief.title, effort: brief.effort, cases: brief.cases, paths: brief.paths,
      shared: brief.shared, depends: brief.depends, brief: path });
    if (existsSync(task.worktree)) copyFileSync(absolute, join(task.worktree, briefFile({ ...task, shared: !!task.worktreeName })));
    console.log(`${task.id} claims replaced from ${path}`);
  });
}

/** Removes a worktree whose processes may still hold files: Windows refuses to delete an open file. */
function removeWorktree(task: Task): void {
  const root = repoRoot();
  killWorktreeProcesses(task.worktree);
  try {
    preserveWorktreeArtifacts(task.worktree, join(stateDir(), 'runs', task.id));
  } catch (error) {
    console.error(`warning: kept ${task.worktree}/.temp in place: ${(error as Error).message}`);
  }
  git(root, ['worktree', 'remove', '--force', task.worktree], true);
  if (existsSync(task.worktree)) {
    // A large Cargo target/ or node_modules can outlast git's removal (long paths, read-only files).
    try { rmSync(task.worktree, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 }); } catch { /* reported below */ }
  }
  git(root, ['worktree', 'prune'], true);
  if (existsSync(task.worktree)) console.error(`warning: ${task.worktree} could not be removed completely; delete it once nothing holds its files`);
}

/** Tracked Markdown under `dirs` (except `skip`) rewritten so its links follow `move`. A file with uncommitted edits is
 * reported instead of rewritten, so an archive commit never takes a manager's work in progress. */
function relinkTracked(root: string, dirs: readonly string[], move: Mover, skip: ReadonlySet<string>):
  { write: { path: string; content: string }[]; notes: string[] } {
  const files = nulList(gitRaw(root, ['ls-files', '-z', '--', ...dirs.map(dir => `:(glob)${dir}/**/*.md`)], true))
    .filter(path => !skip.has(path) && existsSync(join(root, path)));
  if (!files.length) return { write: [], notes: [] };
  const dirty = new Set(parsePorcelainZ(gitRaw(root, ['status', '--porcelain', '-z', '--', ...files], true)));
  const write: { path: string; content: string }[] = [];
  const notes: string[] = [];
  for (const path of files) {
    const content = readFileSync(join(root, path), 'utf8');
    const relinked = relinkMarkdown(content, path, path, move);
    if (relinked === content) continue;
    if (dirty.has(path)) notes.push(`${path} links to a file that moved to ${ARCHIVE_DIR}/; it has uncommitted edits, so fix its links by hand`);
    else write.push({ path, content: relinked });
  }
  return { write, notes };
}

/** Moves the briefs and handoffs of closed tasks to archive/goals and removes the briefs from the tree in one commit.
 * Links in the moved briefs, and links to them from Goal files and the archive, are rewritten to keep resolving.
 * Briefs that tracked files still cite are kept and reported: the citing file must point at an owner first.
 * Idempotent, so `tidy` repairs a close that stopped part-way. */
function archiveClosedBriefs(ledger: Ledger, tasks: readonly Task[]): string[] {
  const root = repoRoot();
  const notes: string[] = [];
  const moved = new Map<string, string>();
  const candidates = tasks.filter(task => !isOpen(task) && task.brief.startsWith(`${GOALS_DIR}/`)
    && existsSync(join(root, task.brief)));
  // One search for every candidate: the cost follows the tree once, not once per brief.
  const citations = treeMentions(root, candidates.map(task => task.brief),
    { exclude: [`${GOALS_DIR}/**`, 'scripts/goal/**', 'archive/**'] });
  const kept: Task[] = [];
  for (const task of candidates) {
    const cited = citations.filter(line => line.includes(task.brief));
    if (cited.length) { notes.push(`${task.id}: kept; cited by\n    ${cited.join('\n    ')}`); continue; }
    moved.set(task.brief, `${archiveDirOf(ledger, task.goal)}/tasks/${task.id}.md`);
    kept.push(task);
  }
  if (!kept.length) return notes;
  if (git(root, ['symbolic-ref', '--short', 'HEAD']) !== 'main') throw new Error('Main checkout is not on main');
  // A brief archived earlier is no longer in its Goal's tasks/ directory but in the archive.
  const move: Mover = path => {
    if (moved.has(path)) return moved.get(path);
    const goal = goalOfBriefPath(path);
    if (!goal || existsSync(join(root, path))) return undefined;
    const archived = `${archiveDirOf(ledger, goal)}/tasks/${basename(path)}`;
    return existsSync(join(root, archived)) ? archived : undefined;
  };
  const write: { path: string; content: string }[] = [];
  for (const task of kept) {
    const to = moved.get(task.brief)!;
    write.push({ path: to, content: relinkMarkdown(readFileSync(join(root, task.brief), 'utf8'), task.brief, to, move) },
      { path: `${posix.dirname(posix.dirname(to))}/handoffs/${task.id}.md`, content: handoffText(task) });
  }
  const relinked = relinkTracked(root, [GOALS_DIR, ARCHIVE_DIR], move, new Set(moved.keys()));
  notes.push(...relinked.notes);
  const subject = kept.length === 1 ? `Archive the closed brief ${kept[0]!.id}` : `Archive ${kept.length} closed briefs`;
  commitPaths(root, [...write, ...relinked.write], [...moved.keys()],
    `${subject}\n\nBriefs and handoffs moved to ${ARCHIVE_DIR}/.\n\n${commitTrailers(new Set(kept.map(task => task.goal)))}`);
  notes.push(`${subject} (now under ${ARCHIVE_DIR}/)`);
  return notes;
}

/** Closes the tasks in order, stopping at the first that cannot close, then archives the briefs and handoffs of those
 * closed in one commit. */
async function closeTasks(ids: string[], outcome: string): Promise<void> {
  if (outcome !== 'verified' && outcome !== 'cancelled') throw new Error('close needs verified or cancelled');
  if (!ids.length) throw new Error('close needs a task ID');
  const ledger = readLedger();
  for (const id of ids) assertOwner(taskOf(ledger, id));
  try {
    for (const id of ids) await closeTask(id, outcome);
  } finally {
    await withLedger(current => {
      for (const note of archiveClosedBriefs(current, ids.map(id => taskOf(current, id)))) console.log(note);
    });
  }
}

async function closeTask(id: string, outcome: 'verified' | 'cancelled'): Promise<void> {
  const plan = await withLedger(ledger => {
    const task = taskOf(ledger, id);
    if (!isOpen(task)) { console.log(`${task.id} is already ${task.state}`); return undefined; }
    if (running(task)) throw new Error(`${task.id} is still running; stop it first`);
    // A read-only task (no path claims, nothing committed) is verified by its accepted handoff.
    const readOnly = !task.paths.length && ['exited', 'stopped'].includes(task.state) && !changedFiles(task).ahead;
    if (outcome === 'verified' && task.state !== 'merged' && !readOnly) throw new Error(`${task.id} is ${task.state}, not merged`);
    // A shared worktree stays while another open task still works in it.
    const last = !sharersOf(ledger, task).some(other => other.id !== task.id);
    return { task: structuredClone(task), last };
  });
  if (!plan) return;
  // Outside the ledger lock: deleting a worktree with a multi-GB Cargo target takes a while.
  if (plan.last && existsSync(plan.task.worktree)) removeWorktree(plan.task);
  if (plan.last && plan.task.state === 'merged') git(repoRoot(), ['branch', '-d', plan.task.branch], true);
  await withLedger(ledger => {
    const task = taskOf(ledger, id);
    task.state = outcome;
    task.closedAt = new Date().toISOString();
    console.log(`${task.id} ${outcome}; claims released${outcome === 'cancelled' ? `, branch ${task.branch} kept` : ''}`);
  });
}

const valueOf = (args: readonly string[], flag: string): string | undefined => {
  const at = args.indexOf(flag);
  return at >= 0 ? args[at + 1] : undefined;
};

/** Registers a Goal whose file exists, or records its manager's new session after a restart. */
async function startGoal(slug: string, args: string[]): Promise<void> {
  if (!validGoalSlug(slug)) throw new Error(`Goal slug must be lower-case words joined by hyphens: ${slug || '(missing)'}`);
  const manager = valueOf(args, '--manager');
  if (!manager) throw new Error('goal start needs --manager <session name>');
  const caller = process.env.GOAL_ID;
  if (caller && caller !== slug) throw new Error(`GOAL_ID is ${caller}; a manager starts only its own Goal`);
  const root = repoRoot();
  if (!existsSync(join(root, goalFile(slug)))) throw new Error(`${goalFile(slug)} is missing; write the Goal first`);
  await withLedger(ledger => {
    const existing = ledger.goals?.[slug];
    if (existing?.closedAt) throw new Error(`Goal ${slug} closed at ${existing.closedAt}; choose another slug`);
    const own = areasOf([slug])[slug] ?? [];
    const overlaps = areaConflicts(own, slug, areasOf(activeGoals(ledger).filter(other => other !== slug)));
    if (overlaps.length && !args.includes('--allow-area')) {
      throw new Error(`Goal ${slug}'s areas overlap another Goal's:\n  ${overlaps.join('\n  ')}`);
    }
    ledger.goals = { ...ledger.goals, [slug]: { ...existing, manager, startedAt: existing?.startedAt ?? new Date().toISOString(),
      seenHead: existing?.seenHead ?? git(root, ['rev-parse', 'main']) } };
    console.log(`Goal ${slug} ${existing ? 'resumed' : 'started'}; manager ${manager}; ${own.length} area(s)`);
  });
}

/** Ends a Goal once nothing of it remains in the tree: no open task, no brief, no file or line naming its tasks and no
 * link into its directory. Then its directory and ledger entries move to archive/goals in one commit. */
async function closeGoal(slug: string, flags: Set<string>): Promise<number> {
  const caller = process.env.GOAL_ID;
  if (caller && caller !== slug) throw new Error(`Goal ${slug} is not this manager's (GOAL_ID is ${caller})`);
  const root = repoRoot();
  return withLedger(ledger => {
    const goal = ledger.goals?.[slug];
    if (!goal || goal.closedAt) throw new Error(`Goal ${slug} is not active`);
    const tasks = Object.values(ledger.tasks).filter(task => task.goal === slug);
    const problems = tasks.filter(isOpen).map(task => `${task.id} is ${task.state}; merge and close or cancel it`);
    if (!problems.length && !flags.has('--dry-run')) for (const note of archiveClosedBriefs(ledger, tasks)) console.log(note);
    const dir = `${GOALS_DIR}/${slug}`;
    const tasksDir = join(root, dir, 'tasks');
    problems.push(...(existsSync(tasksDir) ? readdirSync(tasksDir) : []).map(name => `${dir}/tasks/${name} remains`));
    const ids = tasks.map(task => task.id);
    problems.push(...treeMentions(root, ids, { words: true }).map(line => `names a task: ${line}`));
    const numbers = new Set(ids.map(id => id.slice(2)));
    const exempt = HISTORY_EXEMPT.map(pattern => new Bun.Glob(pattern));
    problems.push(...nulList(gitRaw(root, ['ls-files', '-z'])).filter(path => !exempt.some(glob => glob.match(path)) && path.split('/')
      .some(segment => numbers.has(/^g-(\d{3,})(?!\d)/i.exec(segment)?.[1] ?? ''))).map(path => `named after a task: ${path}`));
    problems.push(...treeMentions(root, [`goals/${slug}/`], { exclude: [`${GOALS_DIR}/${slug}/**`, 'scripts/goal/**', 'archive/**'] })
      .map(line => `links into the Goal: ${line}`));
    if (problems.length) {
      console.log(`Goal ${slug} has not converged:\n  ${problems.join('\n  ')}`);
      return 1;
    }
    const archive = archiveDirOf(ledger, slug);
    if (flags.has('--dry-run')) { console.log(`Goal ${slug} has converged; close would move ${dir}/ to ${archive}/`); return 0; }
    const files = [...nulList(gitRaw(root, ['ls-files', '-z', '--', dir])),
      ...nulList(gitRaw(root, ['ls-files', '-z', '--others', '--exclude-standard', '--', dir]))];
    // Links follow the move: the Goal's own files to each other, and archived briefs back to GOAL.md.
    const move: Mover = path => path === dir || path.startsWith(`${dir}/`) ? archive + path.slice(dir.length) : undefined;
    const moved = files.map(path => {
      const to = move(path)!;
      return { path: to, content: path.endsWith('.md')
        ? relinkMarkdown(readFileSync(join(root, path), 'utf8'), path, to, move) : readFileSync(join(root, path)) };
    });
    const relinked = relinkTracked(root, [ARCHIVE_DIR], move, new Set());
    for (const note of relinked.notes) console.log(note);
    commitPaths(root, [...moved, ...relinked.write,
      { path: `${archive}/ledger.json`, content: `${JSON.stringify({ goal: { slug, ...goal }, tasks }, null, 2)}\n` }],
    files, `Close Goal ${slug}\n\nIts directory and ledger entries moved to ${archive}/.\n\n${commitTrailers([slug])}`);
    rmSync(join(root, dir), { recursive: true, force: true });
    ledger.lastId = Math.max(ledger.lastId ?? 0, ...ids.map(id => Number(id.slice(2))));
    for (const id of ids) delete ledger.tasks[id];
    Object.assign(goal, { closedAt: new Date().toISOString(), archive });
    console.log(`Goal ${slug} closed; ${archive}/ holds its directory, briefs, handoffs and ledger`);
    return 0;
  });
}

/** Task IDs on disk: open briefs in every Goal directory and archived ones, so a lost ledger never reissues an ID. */
function idsOnDisk(root: string): string[] {
  const ids: string[] = [];
  for (const base of [GOALS_DIR, ARCHIVE_DIR]) {
    const dir = join(root, base);
    for (const entry of existsSync(dir) ? readdirSync(dir) : []) {
      const tasks = join(dir, entry, 'tasks');
      if (existsSync(tasks) && statSync(tasks).isDirectory()) ids.push(...readdirSync(tasks).map(name => name.replace(/\.md$/, '')));
    }
  }
  return ids;
}

/** Reserves the next task ID for a Goal and writes its brief skeleton, so two managers never take the same number. */
async function newBrief(args: string[]): Promise<void> {
  const goalArg = valueOf(args, '--goal');
  const title = args.filter((arg, at) => arg !== '--goal' && args[at - 1] !== '--goal').join(' ').trim();
  if (!title) throw new Error('new needs a title');
  const root = repoRoot();
  await withLedger(ledger => {
    const active = activeGoals(ledger);
    const goal = goalArg ?? process.env.GOAL_ID ?? (active.length === 1 ? active[0] : undefined);
    if (!goal || !active.includes(goal)) throw new Error(`new needs an active Goal (--goal <slug>): ${active.join(', ') || 'none started'}`);
    const id = nextTaskId([...Object.keys(ledger.tasks), ...Object.keys(ledger.reserved ?? {}), ...idsOnDisk(root)], ledger.lastId);
    const path = join(root, goalBriefFile(goal, id));
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, ['---', `id: ${id}`, `title: ${title}`,
      `engine: ${DEFAULT_ENGINE}                         # ${ENGINES.join(' | ')}`, 'effort: high', 'cases: []',
      'paths: []                             # name new files by capability, never g-NNN',
      'shared: []                            # e.g. adr:0012 to reserve an ADR number', 'depends: []', '---', '',
      '## 结果', '', '## 检查', ''].join('\n'));
    ledger.reserved = { ...ledger.reserved, [id]: goal };
    console.log(`${id} reserved for Goal ${goal}: ${goalBriefFile(goal, id)}`);
  });
}

/** Archives the briefs of closed tasks still in the tree: a close that stopped part-way. */
async function tidy(): Promise<void> {
  const caller = process.env.GOAL_ID;
  await withLedger(ledger => {
    const tasks = Object.values(ledger.tasks).filter(task => !caller || task.goal === caller);
    const notes = archiveClosedBriefs(ledger, tasks);
    console.log(notes.length ? notes.join('\n') : 'Nothing to tidy');
  });
}

const slotsDir = (): string => join(stateDir(), 'slots');
const slotCount = (): number => Math.max(1, Number(process.env.GOAL_QA_SLOTS || 2));

function describeHolder(owner: LockOwner): string {
  return `Goal ${owner.goal ?? '?'} pid ${owner.pid} since ${owner.acquiredAt}: ${owner.command ?? ''}`;
}

function diskFree(path: string): string {
  try {
    const stats = statfsSync(path);
    return `${Math.round(Number(stats.bavail) * Number(stats.bsize) / 1e9)} GB free`;
  } catch { return 'free space unknown'; }
}

async function status(): Promise<void> {
  const root = repoRoot();
  const ledger = await withLedger(current => {
    for (const task of Object.values(current.tasks)) {
      if (task.state === 'running' && !running(task)) {
        task.state = 'exited';
        lastAttempt(task).endedAt ??= new Date().toISOString();
      }
    }
    return current;
  });
  const tasks = Object.values(ledger.tasks);
  const live = tasks.filter(running);
  const worktrees = existsSync(join(root, '.temp', 'worktrees')) ? readdirSync(join(root, '.temp', 'worktrees')).length : 0;
  console.log(`program elapsed ${ledger.startedAt ? elapsed(ledger.startedAt) : 'not started'}; live ${live.length}/${maxWorkers()};`
    + ` ${worktrees} worktree(s), each with its own Cargo target; disk ${diskFree(root)}`);
  const heavy = lockHolder(join(slotsDir(), 'heavy'));
  const light = Array.from({ length: slotCount() }, (_, k) => lockHolder(join(slotsDir(), `light-${k}`))).filter(Boolean) as LockOwner[];
  console.log(`check slots: ${light.length}/${slotCount()} busy; heavy ${heavy ? describeHolder(heavy) : 'free'}`);
  for (const owner of light) console.log(`  slot: ${describeHolder(owner)}`);
  if (tasks.some(task => isOpen(task) && engineOf(task) === 'codex')) console.log(describeCodex(codexUsage()));
  for (const slug of activeGoals(ledger)) {
    const own = tasks.filter(task => task.goal === slug);
    console.log(`Goal ${slug}: manager ${managerOf(ledger, slug)}; ${own.filter(running).length} live, ${own.filter(isOpen).length} open`);
  }
  for (const task of tasks.filter(isOpen)) {
    const attempt = lastAttempt(task);
    console.log(`${task.id} ${task.state.padEnd(8)} ${task.goal} ${engineOf(attempt)}/${attempt.effort} #${attempt.n} `
      + `${elapsed(attempt.startedAt)} [${task.cases.join(' ')}] ${task.title}`);
  }
  const closed = tasks.filter(task => !isOpen(task)).length;
  if (closed) console.log(`${closed} closed task(s) omitted`);
}

/** Token use and cost per task and engine, from the attempts' recorded output. */
function usageReport(ids: string[]): Record<string, unknown> {
  const ledger = readLedger();
  const tasks = ids.length ? ids.map(id => taskOf(ledger, id)) : Object.values(ledger.tasks);
  const totals: Record<string, { attempts: number; cost: number } & TokenUse> = {};
  const rows = tasks.map(task => ({ id: task.id, goal: task.goal, state: task.state, attempts: task.attempts.map(attempt => {
    const result = readResult(attempt);
    const engine = engineOf(attempt);
    const total = totals[engine] ??= { attempts: 0, cost: 0, input: 0, cached: 0, output: 0 };
    total.attempts += 1;
    total.cost += result.cost ?? 0;
    total.input += result.usage?.input ?? 0;
    total.cached += result.usage?.cached ?? 0;
    total.output += result.usage?.output ?? 0;
    return { n: attempt.n, engine, model: modelOf(engine), effort: attempt.effort, error: result.error, cost: result.cost,
      ...result.usage, minutes: attempt.endedAt ? Math.round((Date.parse(attempt.endedAt) - Date.parse(attempt.startedAt)) / 60_000) : undefined };
  }) }));
  return { tasks: rows, totals, codex: codexUsage() };
}

/** Runs a check command in one of the host's light slots; heavy runs first take the host-wide heavy lock, so the host
 * carries at most one heavy run beside the light ones. Both locks belong to this process and free themselves when it
 * exits, also when it crashes. */
async function withSlot(command: string[], heavy: boolean): Promise<number> {
  if (!command.length) throw new Error('slot needs a command: slot [--heavy] -- <command>');
  if (/\.ps1$/i.test(command[0]!)) command = [powershell(), '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ...command];
  heavy ||= isHeavyCommand(command);
  const nested = process.env.GOAL_IN_SLOT === '1';
  const info = { goal: process.env.GOAL_ID ?? process.env.GOAL_TASK_ID, command: command.join(' ') };
  const releases: (() => void)[] = [];
  try {
    // A command already inside a slot (a gate that calls another check) runs in its parent's slot.
    if (!nested) {
      // Take the heavy lock before a slot, so a waiting heavy run never holds a slot that light runs need.
      if (heavy) releases.push(await acquireLock(join(slotsDir(), 'heavy'), { timeoutMs: 6 * 3_600_000, pollMs: 10_000, label: 'the heavy check lock', info }));
      const deadline = Date.now() + 3_600_000;
      let announced = false;
      for (;;) {
        let release: (() => void) | undefined;
        for (let k = 0; k < slotCount() && !release; k++) release = tryLock(join(slotsDir(), `light-${k}`), info);
        if (release) { releases.push(release); break; }
        if (Date.now() > deadline) throw new Error('No check slot became free within one hour');
        if (!announced) { console.error(`waiting for one of ${slotCount()} check slots`); announced = true; }
        await Bun.sleep(3000);
      }
    }
    // No windowsHide: the command shares the caller's console (a terminal, or a worker's hidden one).
    const child = spawn(command[0]!, command.slice(1), { cwd: process.cwd(), stdio: 'inherit',
      env: { ...process.env, GOAL_IN_SLOT: '1' } });
    const stop = (): void => { if (child.pid) killTree(child.pid); };
    for (const signal of ['SIGINT', 'SIGTERM'] as const) { try { process.on(signal, stop); } catch { /* unsupported */ } }
    return await new Promise<number>(done => {
      child.on('error', error => { console.error(`${command[0]}: ${error.message}`); done(127); });
      child.on('exit', code => done(code ?? 1));
    });
  } finally {
    for (const release of releases.reverse()) release();
  }
}

/** Sign-in state of every engine CLI, without starting a model. */
function authReport(): void {
  for (const engine of ['claude', 'codex', 'grok'] as const) {
    try {
      preflight(engine, repoRoot());
      console.log(`${engine}: signed in`);
    } catch (error) {
      console.log((error as Error).message.split('; choose')[0]);
    }
  }
}

const USAGE = 'Usage: goalctl goal start <slug> --manager <session> [--allow-area] | goal close <slug> [--dry-run]'
  + ' | new [--goal <slug>] <title> | dispatch <brief.md> [--dry-run] [--allow-area] [--skip-preflight] [--force-usage]'
  + ' | wait <id> | resume <id> (-m <text> | --file <path>) [--effort e] [--engine e] [--fresh] [--skip-preflight]'
  + ' | reclaim <id> <brief> | stop <id> | scope <id> | owner <path> | merge <id> [--allow-scope] [--allow-ids] [--landed]'
  + ' | close <id>... verified|cancelled | tidy | status | usage [<id>...] | auth | slot [--heavy] -- <command>'
  + `\nEngines: ${ENGINES.map(engine => `${engine} (${modelOf(engine)})`).join(', ')}`;

export async function main(argv: string[]): Promise<number> {
  const [command, ...rest] = argv;
  const flags = new Set(rest.filter(arg => arg.startsWith('--')));
  const positional = rest.filter(arg => !arg.startsWith('--'));
  switch (command) {
    case 'goal': {
      const [action, slug = ''] = positional;
      if (action === 'start') { await startGoal(slug, rest.slice(2)); return 0; }
      if (action === 'close') return closeGoal(slug, flags);
      throw new Error('goal needs start <slug> --manager <session> [--allow-area] | close <slug> [--dry-run]');
    }
    case 'new': await newBrief(rest); return 0;
    case 'tidy': await tidy(); return 0;
    case 'dispatch': await dispatch(positional[0] ?? '', flags); return 0;
    case 'wait': await waitFor(positional[0] ?? ''); return 0;
    case 'resume': await resumeTask(rest[0] ?? '', rest.slice(1)); return 0;
    case 'stop': await stopTask(positional[0] ?? ''); return 0;
    case 'scope': {
      const ledger = readLedger();
      const task = taskOf(ledger, positional[0] ?? '');
      console.log(describe(task, sharersOf(ledger, task).flatMap(member => member.paths)));
      return 0;
    }
    case 'merge': await mergeTask(positional[0] ?? '', flags); return 0;
    case 'close': await closeTasks(positional.slice(0, -1), positional.at(-1) ?? ''); return 0;
    case 'reclaim': await reclaimTask(positional[0] ?? '', positional[1] ?? ''); return 0;
    case 'owner': {
      // Read-only: which open task claims a repository path (workers check before editing outside their claim).
      const path = forward(positional[0] ?? '');
      const ledger = readLedger();
      const holders = Object.values(ledger.tasks).filter(task => HOLDING.includes(task.state)
        && task.paths.some(pattern => new Bun.Glob(pattern).match(path) || pathsOverlap(pattern, path)));
      const areas = Object.entries(areasOf(activeGoals(ledger)))
        .filter(([, globs]) => globs.some(glob => pathsOverlap(glob, path))).map(([slug]) => slug);
      console.log((holders.length ? `${path}: claimed by ${holders.map(task => `${task.id} (${task.state})`).join(', ')}`
        : `${path}: unclaimed`) + (areas.length ? `; in Goal ${areas.join(', ')}'s area` : ''));
      return holders.length ? 1 : 0;
    }
    case 'status': await status(); return 0;
    case 'usage': console.log(JSON.stringify(usageReport(positional), null, 2)); return 0;
    case 'auth': authReport(); return 0;
    case '__run': return runWorker(positional[0] ?? '');
    case 'slot': {
      const heavy = rest[0] === '--heavy';
      const remainder = heavy ? rest.slice(1) : rest;
      return withSlot(remainder[0] === '--' ? remainder.slice(1) : remainder, heavy);
    }
    default:
      console.error(USAGE);
      return 2;
  }
}

if (import.meta.main) {
  try {
    process.exit(await main(process.argv.slice(2)));
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
}
