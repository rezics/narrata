import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, utimesSync, writeFileSync } from 'node:fs';
import { devNull, tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { afterAll, describe, expect, test } from 'bun:test';
import { areaConflicts, authProblem, briefFile, claimConflicts, CODEX_MODEL, commitPaths, commitTrailers, failureHint,
  goalAreas, goalOfBriefPath, historyIntroductions, isHeavyCommand, killTree, killWorktreeProcesses, launchCommand,
  lockHolder, MANAGER_SESSION_ENV, maintainerDocCommits, modelOf, nextTaskId, outOfScope, ownerRefusal, parseBrief,
  parseCodexUsage, parsePorcelainZ, parseResumeArgs, pathsOverlap, preserveWorktreeArtifacts, processInfo, promptOnStdin, readResult,
  relinkMarkdown, sameProcess, type Task, terminate, treeMentions, tryLock, validateBrief, workerEnv,
  worktreeProcessPattern } from './goalctl.ts';
// The repository's own link checker (`task docs:check`), which also reads archive/.
import { checkFile } from '../docs/check.ts';

const GOALCTL = join(import.meta.dir, 'goalctl.ts');
const FAKE_CLI = join(import.meta.dir, 'fake-cli.ts');
const windows = process.platform === 'win32';
const scratch: string[] = [];
const spawned: number[] = [];

function tempDir(prefix: string): string {
  const dir = mkdtempSync(join(tmpdir(), `goalctl-${prefix}-`));
  scratch.push(dir);
  return dir;
}

afterAll(() => {
  for (const pid of spawned) if (processInfo(pid).alive) killTree(pid);
  for (const dir of scratch) {
    try { rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }); } catch { /* a held file */ }
  }
});

const brief = `---
id: G-040
title: Choice placement anchors  # comment
engine: claude
effort: xhigh
cases: [CH01, CH02]
paths: [packages/narrata/nodes/crates/narrata-nodes/src/choice/**, packages/narrata/nodes/crates/narrata-nodes/tests/choice-*.rs]
migrations: []
shared: [adr:0012]
depends: [G-038]
---
Body.
`;

function held(overrides: Partial<Task>): Task {
  return { id: 'G-039', title: 'held', effort: 'medium', engine: 'claude', cases: [], paths: [], shared: [], depends: [],
    brief: '', worktree: '', branch: '', base: '', state: 'running', attempts: [], goal: 'narrative-core', ...overrides };
}

const briefWith = (engine: string, effort: string): ReturnType<typeof parseBrief> =>
  parseBrief(`---\nid: G-081\ntitle: t\neffort: ${effort}\nengine: ${engine}\n---\n`);

describe('briefs', () => {
  test('parses frontmatter lists, strips comments and accepts an empty migrations list', () => {
    const parsed = parseBrief(brief);
    expect(parsed).toMatchObject({ id: 'G-040', title: 'Choice placement anchors', effort: 'xhigh', engine: 'claude',
      cases: ['CH01', 'CH02'], shared: ['adr:0012'], depends: ['G-038'], migrations: [] });
    expect(validateBrief(parsed)).toEqual([]);
  });

  test('reads CRLF briefs written on Windows', () => {
    expect(parseBrief(brief.replaceAll('\n', '\r\n')).paths).toHaveLength(2);
  });

  test('rejects reserved migration ranges, backslash, absolute and .temp paths', () => {
    const errors = validateBrief(parseBrief(brief.replace('migrations: []', 'migrations: [main/access:040-044]')));
    expect(errors.join('\n')).toContain('migrations: Narrata has no numbered migrations');
    const paths = (path: string) => validateBrief({ ...parseBrief(brief), paths: [path] });
    expect(paths('crates\\narrata-core\\**').join()).toContain('forward slashes');
    expect(paths('D:/rezics-repos/narrata/crates/**').join()).toContain('repository-relative');
    expect(paths('/etc/**').join()).toContain('repository-relative');
    expect(paths('.temp/x').join()).toContain('repository-relative');
    expect(paths('crates/../x').join()).toContain('repository-relative');
  });

  test('accepts the efforts each engine supports and rejects removed engines', () => {
    expect(validateBrief(briefWith('claude', 'max'))).toEqual([]);
    expect(validateBrief(briefWith('sonnet', 'low'))).toEqual([]);
    expect(validateBrief(briefWith('fable', 'xhigh'))).toEqual([]);
    expect(validateBrief(briefWith('codex', 'ultra'))).toEqual([]);
    expect(validateBrief(briefWith('grok', 'high'))).toEqual([]);
    expect(validateBrief(briefWith('claude', 'ultra')).join()).toContain('claude effort');
    expect(validateBrief(briefWith('grok', 'xhigh')).join()).toContain('grok effort');
    for (const removed of ['codex-1', 'luna', 'cursor']) expect(validateBrief(briefWith(removed, 'high')).join()).toContain('engine must be one of');
  });

  test('case IDs are upper-case codes ending in a digit; task IDs are not cases', () => {
    expect(validateBrief({ ...parseBrief(brief), cases: ['R1', 'GB01'] })).toEqual([]);
    expect(validateBrief({ ...parseBrief(brief), cases: ['G-001'] })).toEqual(['bad case ID: G-001']);
  });

  test('a brief may name a shared worktree; its brief file is per task', () => {
    const shared = parseBrief('---\nid: G-950\ntitle: t\neffort: high\nworktree: wave-9\npaths: [a/**]\n---\n');
    expect(shared.worktree).toBe('wave-9');
    expect(validateBrief(shared)).toEqual([]);
    expect(validateBrief({ ...shared, worktree: '../x' })).toContain('worktree must be a lower-case name: ../x');
    expect(briefFile({ id: 'G-950', shared: true })).toBe('.temp/goal/brief-g-950.md');
    expect(briefFile({ id: 'G-950' })).toBe('.temp/goal/brief.md');
  });
});

describe('claims', () => {
  const parsed = parseBrief(brief);

  test('detects case, path and shared-slot overlap with holding tasks only', () => {
    const conflicts = claimConflicts(parsed, [
      held({ cases: ['CH02'], paths: ['packages/narrata/nodes/crates/narrata-nodes/tests/choice-anchor.rs'], shared: ['adr:0012'] }),
      held({ id: 'G-030', cases: ['CH01'], state: 'verified' }),
    ]);
    expect(conflicts).toHaveLength(3);
    expect(conflicts.join('\n')).not.toContain('G-030');
  });

  test('keeps disjoint claims independent', () => {
    expect(claimConflicts(parsed, [held({ cases: ['CH03'], paths: ['packages/narrata/nodes/crates/narrata-nodes/src/choice.rs'],
      shared: ['adr:0013'] })])).toEqual([]);
    expect(pathsOverlap('crates/narrata-core/**', 'crates/narrata-store/**')).toBe(false);
    expect(pathsOverlap('crates/**', 'crates/narrata-store/src/lib.rs')).toBe(true);
    expect(pathsOverlap('scripts/check-g*.ps1', 'scripts/check-r1.ps1')).toBe(false);
    expect(pathsOverlap('scripts/check-g*.ps1', 'scripts/check-g5.ps1')).toBe(true);
    expect(pathsOverlap('crates/*/tests/*sqlite*.rs', 'crates/narrata-store-sqlite/tests/sqlite.rs')).toBe(true);
    expect(pathsOverlap('crates/**/model.rs', 'crates/narrata-store/src/lib.rs')).toBe(false);
    expect(pathsOverlap('examples/gamebook-web/src/[a-z]*/**', 'examples/gamebook-web/src/reader/App.tsx')).toBe(true);
    expect(pathsOverlap('fixtures/compat/stage5-v0/**', 'fixtures/compat/stage6-v0/**')).toBe(false);
  });

  test('reports changed files outside the claimed globs', () => {
    expect(outOfScope(['packages/narrata/nodes/crates/narrata-nodes/src/choice/anchor.rs', 'Cargo.toml'], parsed.paths))
      .toEqual(['Cargo.toml']);
  });
});

describe('Goals', () => {
  test('a brief belongs to the Goal whose tasks directory holds it', () => {
    expect(goalOfBriefPath('docs/goals/narrative-core/tasks/G-001.md')).toBe('narrative-core');
    expect(goalOfBriefPath('docs\\goals\\narrative-core\\tasks\\G-001.md')).toBe('narrative-core');
    expect(goalOfBriefPath('docs/goals/tasks/G-001.md')).toBeUndefined();
    expect(goalOfBriefPath('docs/goals/narrative-core/GOAL.md')).toBeUndefined();
  });

  test('another active Goal\'s areas refuse a claim; the own Goal\'s and unowned paths do not', () => {
    const goalText = '---\r\n# 其他 Goal 的任务简报不得认领的粗粒度路径\r\nareas:\r\n  - crates/**  # kernel\r\n  - scripts/check-g*.ps1\r\n---\r\n# Kernel\r\n';
    const areas = { 'kernel-and-saves': goalAreas(goalText), 'narrative-core': ['packages/narrata/nodes/**'] };
    expect(areas['kernel-and-saves']).toEqual(['crates/**', 'scripts/check-g*.ps1']);
    expect(areaConflicts(['crates/narrata-core/src/kernel.rs'], 'narrative-core', areas))
      .toEqual(['path crates/narrata-core/src/kernel.rs lies in Goal kernel-and-saves\'s area crates/**']);
    expect(areaConflicts(['crates/**'], 'kernel-and-saves', areas)).toEqual([]);
    expect(areaConflicts(['scripts/check-r1.ps1', 'docs/product/goal.md'], 'narrative-core', areas)).toEqual([]);
    expect(goalAreas('# A Goal without frontmatter\n')).toEqual([]);
  });

  test('IDs start at G-001 and continue after every used, reserved and archived number', () => {
    expect(nextTaskId([])).toBe('G-001');
    expect(nextTaskId(['G-007', 'G-012', 'notes'], 3)).toBe('G-013');
    expect(nextTaskId(['G-999'])).toBe('G-1000');
    expect(nextTaskId(['G-002'], 41)).toBe('G-042');
  });

  test('a manager may change only its own Goal\'s tasks', () => {
    expect(ownerRefusal({ id: 'G-010', goal: 'narrative-core' }, 'kernel-and-saves')).toContain('belongs to Goal narrative-core');
    expect(ownerRefusal({ id: 'G-010', goal: 'narrative-core' }, 'narrative-core')).toBeUndefined();
    // An explicit undefined would fall back to the caller's own GOAL_ID, so an unset one is passed as ''.
    expect(ownerRefusal({ id: 'G-010', goal: 'narrative-core' }, '')).toBeUndefined();
  });
});

describe('history gate', () => {
  test('refuses new task-named files and task IDs a file did not carry', () => {
    expect(historyIntroductions([
      { path: 'crates/narrata-core/tests/g-012-replay.rs', status: 'A', after: '#[test] fn replay() {}' },
      { path: 'crates/narrata-core/src/flow.rs', status: 'M', before: '// G-003 keeps it', after: '// G-003 keeps it\n// see G-012' },
    ])).toEqual([
      'crates/narrata-core/tests/g-012-replay.rs: named after a task; name it by the capability it covers',
      'crates/narrata-core/src/flow.rs: adds G-012; state the reason itself instead of citing the task',
    ]);
  });

  test('allows moves, renames away from task names and history in the Goal program and archive', () => {
    expect(historyIntroductions([
      { path: 'crates/narrata-core/src/a.rs', status: 'M', before: 'x // G-003\ny', after: 'y\nx // G-003' },
      { path: 'crates/narrata-core/tests/replay.rs', status: 'R', from: 'crates/narrata-core/tests/g-003-replay.rs' },
      { path: 'docs/goals/narrative-core/tasks/G-013.md', status: 'A', after: 'depends: [G-012]' },
      { path: 'archive/goals/narrative-core-2026-10-05/handoffs/G-012.md', status: 'A', after: 'G-012' },
      { path: 'scripts/goal/goalctl.test.ts', status: 'M', before: '', after: "id: 'G-012'" },
      { path: 'products/gamebook-demo/cover.png', status: 'A', after: 'PNG\0G-012' },
      { path: 'crates/narrata-core/src/b.rs', status: 'D', before: 'G-1' },
    ])).toEqual([]);
  });
});

describe('resume', () => {
  test('keeps every -m text and --file content in argument order', () => {
    const read = (path: string) => `review in ${path}`;
    expect(parseResumeArgs(['--file', 'r.md', '-m', 'apply it'], read).message).toBe('review in r.md\n\napply it');
    expect(parseResumeArgs(['-m', 'first', '--effort', 'high'], read)).toMatchObject({ message: 'first', effort: 'high' });
    expect(() => parseResumeArgs(['-m', '  '], read)).toThrow('resume needs');
  });
});

describe('engines', () => {
  const restore = (key: string, value: string | undefined): void => {
    if (value === undefined) delete process.env[key];
    else process.env[key] = value;
  };

  test('Claude Code workers pin model and effort, run in bypass mode and read the prompt from stdin', () => {
    for (const [engine, model] of [['claude', 'claude-opus-5-5'], ['fable', 'claude-fable-5-1'], ['sonnet', modelOf('sonnet')]] as const) {
      const [program, args] = launchCommand({ id: 'G-040', effort: 'high', session: 's', prompt: 'SECRET PROMPT', resume: false, engine });
      expect(program).toBe('claude');
      expect(args).toEqual(['-p', '--model', model, '--effort', 'high', '--dangerously-skip-permissions',
        '--session-id', 's', '-n', 'g-040', '--output-format', 'json']);
      expect(promptOnStdin(engine)).toBe(true);
    }
    expect(launchCommand({ id: 'G-040', effort: 'max', session: 's', prompt: 'p', resume: true })[1])
      .toEqual(expect.arrayContaining(['--resume', 's']));
    const previous = process.env.GOAL_SONNET_MODEL;
    try {
      delete process.env.GOAL_SONNET_MODEL;
      expect(modelOf('sonnet')).toBe('claude-sonnet-5-5');
      process.env.GOAL_SONNET_MODEL = 'claude-sonnet-5';
      expect(modelOf('sonnet')).toBe('claude-sonnet-5');
    } finally { restore('GOAL_SONNET_MODEL', previous); }
  });

  test('Codex runs GPT-6.1 Sol in its worktree, with the prompt on stdin and the last message in a file', () => {
    const previous = [process.env.GOAL_CODEX_MODEL, process.env.GOAL_CODEX_SERVICE_TIER] as const;
    try {
      delete process.env.GOAL_CODEX_MODEL;
      delete process.env.GOAL_CODEX_SERVICE_TIER;
      expect(CODEX_MODEL).toBe('gpt-6.1-sol');
      const [program, args] = launchCommand({ id: 'G-081', effort: 'ultra', session: '', prompt: 'p', resume: false,
        engine: 'codex', worktree: 'D:\\w', lastMessage: 'D:\\r\\last.md' });
      expect(program).toBe('codex');
      expect(args).toEqual(['exec', '-m', 'gpt-6.1-sol', '-c', 'model_reasoning_effort=ultra',
        '--dangerously-bypass-approvals-and-sandbox', '--json', '-o', 'D:\\r\\last.md', '-C', 'D:\\w', '-']);
      expect(launchCommand({ id: 'G-081', effort: 'high', session: 't', prompt: 'p', resume: true, engine: 'codex' })[1])
        .toEqual(['exec', 'resume', 't', '-m', 'gpt-6.1-sol', '-c', 'model_reasoning_effort=high',
          '--dangerously-bypass-approvals-and-sandbox', '--json', '-o', devNull, '-']);
      process.env.GOAL_CODEX_MODEL = 'gpt-6-astra';
      process.env.GOAL_CODEX_SERVICE_TIER = 'fast';
      const [, overridden] = launchCommand({ id: 'G-081', effort: 'high', session: '', prompt: 'p', resume: false, engine: 'codex' });
      expect(overridden.join(' ')).toContain('-m gpt-6-astra');
      expect(overridden).toContain('service_tier="fast"');
    } finally {
      restore('GOAL_CODEX_MODEL', previous[0]);
      restore('GOAL_CODEX_SERVICE_TIER', previous[1]);
    }
  });

  test('Grok takes the prompt as an argument, bounded by the Windows command line', () => {
    const [program, args] = launchCommand({ id: 'G-095', effort: 'high', session: 's', prompt: 'p', resume: true,
      engine: 'grok', worktree: '/w' });
    expect(program).toBe('grok');
    expect(args).toEqual(['-p', 'p', '-m', 'grok-4.7', '--reasoning-effort', 'high', '--permission-mode', 'bypassPermissions',
      '--no-subagents', '--output-format', 'json', '--cwd', '/w', '-r', 's']);
    expect(promptOnStdin('grok')).toBe(false);
    expect(() => launchCommand({ id: 'G-095', effort: 'high', session: '', prompt: 'x'.repeat(40_000), resume: false, engine: 'grok' }))
      .toThrow('Windows limit');
  });

  test('GOAL_<FAMILY>_COMMAND replaces the executable with a path or an argv prefix', () => {
    const previous = process.env.GOAL_CLAUDE_COMMAND;
    try {
      process.env.GOAL_CLAUDE_COMMAND = JSON.stringify(['bun', 'fake.ts']);
      const [program, args] = launchCommand({ id: 'G-001', effort: 'low', session: 's', prompt: 'p', resume: false });
      expect(program).toBe('bun');
      expect(args.slice(0, 2)).toEqual(['fake.ts', '-p']);
      process.env.GOAL_CLAUDE_COMMAND = 'C:\\tools\\claude.exe';
      expect(launchCommand({ id: 'G-001', effort: 'low', session: 's', prompt: 'p', resume: false })[0]).toBe('C:\\tools\\claude.exe');
    } finally { restore('GOAL_CLAUDE_COMMAND', previous); }
  });

  test('a worker does not inherit the manager\'s Claude Code session', () => {
    const base = Object.fromEntries([...MANAGER_SESSION_ENV.map(key => [key, 'manager']), ['PATH', 'p'],
      ['ANTHROPIC_BASE_URL', 'u'], ['CARGO_HOME', 'c'], ['claudecode', 'x']]);
    expect(workerEnv(base, { GOAL_TASK_ID: 'G-001' })).toEqual({ PATH: 'p', ANTHROPIC_BASE_URL: 'u', CARGO_HOME: 'c', GOAL_TASK_ID: 'G-001' });
  });

  test('reads each CLI\'s sign-in probe', () => {
    expect(authProblem('claude', { status: 1, stdout: '{\n  "loggedIn": false,\n  "authMethod": "none"\n}' }))
      .toBe('claude is not signed in (auth method none)');
    expect(authProblem('sonnet', { status: 0, stdout: '{"loggedIn": true, "authMethod": "claude.ai"}' })).toBeUndefined();
    expect(authProblem('codex', { status: 0, stdout: 'Logged in using ChatGPT' })).toBeUndefined();
    expect(authProblem('codex', { status: 1, stderr: 'Not logged in' })).toContain('not signed in');
    expect(authProblem('grok', { status: 0, stdout: 'You are not authenticated.\n\nDefault model: grok-4.6' }))
      .toBe('grok is not signed in: You are not authenticated.');
    expect(authProblem('grok', { status: 0, stdout: 'Available models:\n  * grok-4.7 (default)' })).toBeUndefined();
    expect(authProblem('codex', { status: null, error: { code: 'ENOENT' } })).toBe('codex is not on PATH');
    expect(authProblem('claude', { status: null, error: { code: 'ETIMEDOUT' } })).toBeUndefined();
  });

  test('names sign-in and usage-limit failures in a worker\'s output', () => {
    expect(failureHint('claude', 'Failed to authenticate: OAuth session expired')).toContain('claude auth login');
    expect(failureHint('grok', 'Error: Not signed in')).toContain('grok login');
    expect(failureHint('codex', 'You have hit your usage limit')).toContain('LIMIT');
    expect(failureHint('claude', 'cargo test failed')).toBeUndefined();
  });

  test('gates, browser suites and workspace builds are heavy; per-crate checks are not', () => {
    expect(isHeavyCommand(['pwsh', '-File', 'scripts/check-r1.ps1'])).toBe(true);
    expect(isHeavyCommand(['powershell', '-File', '.\\scripts\\check-g5.ps1'])).toBe(true);
    expect(isHeavyCommand(['npm', '--prefix', 'examples/gamebook-web', 'run', 'test:e2e'])).toBe(true);
    expect(isHeavyCommand(['npx', 'playwright', 'test'])).toBe(true);
    expect(isHeavyCommand(['cargo', 'test', '--workspace'])).toBe(true);
    expect(isHeavyCommand(['cargo', 'clippy', '--workspace', '--all-targets'])).toBe(true);
    expect(isHeavyCommand(['cargo', 'fmt', '--all', '--check'])).toBe(false);
    expect(isHeavyCommand(['cargo', 'test', '-p', 'narrata-nodes'])).toBe(false);
    expect(isHeavyCommand(['npm', 'run', 'check'])).toBe(false);
    expect(isHeavyCommand(['task', 'check:r1'])).toBe(true);
    expect(isHeavyCommand(['C:\\tools\\task.exe', 'bench:g2'])).toBe(true);
    expect(isHeavyCommand(['pwsh', '-File', 'scripts/benchmark-g2.ps1'])).toBe(true);
    expect(isHeavyCommand(['task', 'docs:check'])).toBe(false);
    expect(isHeavyCommand(['task', 'test:scripts'])).toBe(false);
  });

  test('reads the Codex account usage from its newest rollout line', () => {
    const now = 1_790_520_000_000;
    const line = (used: number, resets: number, reached: string | null = null) => JSON.stringify({
      timestamp: '2026-09-27T14:36:48.837Z', type: 'event_msg', payload: { type: 'token_count', rate_limits: {
        primary: { used_percent: used, window_minutes: 10080, resets_at: resets }, secondary: null,
        plan_type: 'pro', rate_limit_reached_type: reached } } });
    const text = [line(40, 1_791_053_423), '{"type":"event_msg"}', line(45, 1_791_053_423), '{"partial'].join('\r\n');
    expect(parseCodexUsage(text, now)).toMatchObject({ used: 45, windowMinutes: 10080, plan: 'pro', reached: false, resetInHours: 148.2 });
    expect(parseCodexUsage(line(100, 1_791_053_423), now).reached).toBe(true);
    expect(parseCodexUsage(line(80, 1_790_000_000), now)).toMatchObject({ used: 0, reached: false });
    expect(parseCodexUsage('', now)).toEqual({});
  });

  test('reads the handoff, session and token use of Claude Code and Codex attempts', () => {
    const dir = tempDir('result');
    const output = join(dir, 'attempt-1.json');
    writeFileSync(output, `${JSON.stringify({ result: 'RESULT: done', session_id: 'abc', is_error: false, total_cost_usd: 1.5,
      usage: { input_tokens: 3, cache_creation_input_tokens: 7, cache_read_input_tokens: 90, output_tokens: 11 } })}\r\n`);
    expect(readResult({ n: 1, effort: 'high', engine: 'claude', pid: 0, session: '', output, startedAt: '' }))
      .toMatchObject({ text: 'RESULT: done', session: 'abc', error: false, cost: 1.5, usage: { input: 10, cached: 90, output: 11 } });
    writeFileSync(output, JSON.stringify({ result: 'Failed to authenticate: OAuth session expired', is_error: true }));
    writeFileSync(join(dir, 'attempt-1.err'), 'stderr tail');
    const failed = readResult({ n: 1, effort: 'high', engine: 'claude', pid: 0, session: '', output, startedAt: '' });
    expect(failed.error).toBe(true);
    expect(failed.text).toContain('stderr tail');
    const codexOut = join(dir, 'attempt-2.json');
    writeFileSync(codexOut, ['{"type":"thread.started","thread_id":"t-1"}',
      '{"type":"turn.completed","usage":{"input_tokens":5,"cached_input_tokens":2,"output_tokens":4}}'].join('\n'));
    writeFileSync(join(dir, 'attempt-2.last.md'), 'RESULT: partial');
    expect(readResult({ n: 2, effort: 'high', engine: 'codex', pid: 0, session: '', output: codexOut,
      lastMessage: join(dir, 'attempt-2.last.md'), startedAt: '' }))
      .toMatchObject({ text: 'RESULT: partial', session: 't-1', error: false, usage: { input: 5, cached: 2, output: 4 } });
  });
});

describe('Windows processes', () => {
  const sleeper = (...extra: string[]) => {
    const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)', ...extra], { detached: true, stdio: 'ignore', windowsHide: true });
    child.unref();
    spawned.push(child.pid!);
    return child.pid!;
  };

  test('liveness compares the start mark, so a reused PID is not the recorded process', async () => {
    const own = processInfo(process.pid);
    expect(own.alive).toBe(true);
    const pid = sleeper();
    const info = processInfo(pid);
    expect(info.alive).toBe(true);
    expect(processInfo(pid).mark).toBe(info.mark);
    expect(sameProcess(pid, info.mark)).toBe(true);
    if (info.mark !== undefined) expect(sameProcess(pid, info.mark + 1)).toBe(false);
    expect(await terminate(pid, info.mark)).toBe(true);
    expect(processInfo(pid).alive).toBe(false);
    expect(processInfo(0).alive).toBe(false);
  });

  test('killTree ends a worker and the descendants it detached', async () => {
    const dir = tempDir('tree');
    const parent = join(dir, 'parent.ts');
    writeFileSync(parent, `import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';
const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { detached: true, stdio: 'ignore', windowsHide: true });
writeFileSync(${JSON.stringify(join(dir, 'child.pid'))}, String(child.pid));
setInterval(() => {}, 1000);
`);
    const root = spawn(process.execPath, [parent], { detached: true, stdio: 'ignore', windowsHide: true });
    root.unref();
    spawned.push(root.pid!);
    const deadline = Date.now() + 10_000;
    while (!existsSync(join(dir, 'child.pid')) && Date.now() < deadline) await Bun.sleep(50);
    const child = Number(readFileSync(join(dir, 'child.pid'), 'utf8'));
    spawned.push(child);
    expect(processInfo(child).alive).toBe(true);
    expect(await terminate(root.pid!, processInfo(root.pid!).mark)).toBe(true);
    for (let i = 0; i < 40 && processInfo(child).alive; i++) await Bun.sleep(100);
    expect(processInfo(child).alive).toBe(false);
  }, 20_000);

  test('the worktree pattern matches every path spelling and not a longer sibling', () => {
    const pattern = new RegExp(worktreeProcessPattern('D:\\repo\\.temp\\worktrees\\g-100'), 'i');
    expect(pattern.test('"C:\\bun.exe" D:\\repo\\.temp\\worktrees\\g-100\\target\\debug\\x.exe')).toBe(true);
    expect(pattern.test('node d:/repo/.temp/worktrees/g-100/node_modules/vite/bin/vite.js')).toBe(true);
    expect(pattern.test('bash -c "cd /d/repo/.temp/worktrees/g-100 && cargo test"')).toBe(true);
    expect(pattern.test('D:\\repo\\.temp\\worktrees\\g-100')).toBe(true);
    expect(pattern.test('D:\\repo\\.temp\\worktrees\\g-1000\\target\\x.exe')).toBe(false);
  });

  test.if(windows)('sweeps the processes that run from a worktree and spares its siblings', async () => {
    const dir = tempDir('sweep');
    const inside = sleeper(join(dir, 'g-100', 'marker'));
    const sibling = sleeper(join(dir, 'g-1000', 'marker'));
    const killed = killWorktreeProcesses(join(dir, 'g-100'));
    expect(killed).toContain(inside);
    expect(killed).not.toContain(sibling);
    for (let i = 0; i < 40 && processInfo(inside).alive; i++) await Bun.sleep(100);
    expect(processInfo(inside).alive).toBe(false);
    expect(processInfo(sibling).alive).toBe(true);
    killTree(sibling);
  }, 60_000);
});

describe('locks', () => {
  test('a lock held by this process is busy until released', () => {
    const path = join(tempDir('lock'), 'light-0');
    const release = tryLock(path, { goal: 'g', command: 'cargo test' })!;
    expect(release).toBeDefined();
    expect(lockHolder(path)).toMatchObject({ pid: process.pid, goal: 'g', command: 'cargo test' });
    expect(tryLock(path)).toBeUndefined();
    release();
    expect(existsSync(path)).toBe(false);
    const again = tryLock(path);
    expect(again).toBeDefined();
    again!();
  });

  test('a dead holder\'s lock and a lock whose PID now names another process are free', async () => {
    const dir = tempDir('stale');
    const exited = spawnSync(process.execPath, ['-e', 'console.log(process.pid)'], { encoding: 'utf8' });
    const dead = Number(exited.stdout.trim());
    for (const [name, owner] of [['dead', { pid: dead, token: 'a', acquiredAt: '' }],
      ['reused', { pid: process.pid, mark: (processInfo(process.pid).mark ?? 0) - 1000, token: 'b', acquiredAt: '' }]] as const) {
      mkdirSync(join(dir, name));
      writeFileSync(join(dir, name, 'owner.json'), JSON.stringify(owner));
      expect(lockHolder(join(dir, name))).toBeUndefined();
      const release = tryLock(join(dir, name));
      expect(release).toBeDefined();
      expect(lockHolder(join(dir, name))?.token).not.toBe(owner.token);
      release!();
    }
  });

  test('a directory without an owner is busy while young and free once old', () => {
    const path = join(tempDir('orphan'), 'heavy');
    mkdirSync(path);
    writeFileSync(join(path, 'partial'), '');
    expect(tryLock(path)).toBeUndefined();
    const old = new Date(Date.now() - 60_000);
    utimesSync(path, old, old);
    const release = tryLock(path);
    expect(release).toBeDefined();
    release!();
  });

  test('a holder killed without cleanup frees its lock', async () => {
    const dir = tempDir('crash');
    const path = join(dir, 'heavy');
    const holder = join(dir, 'holder.ts');
    writeFileSync(holder, `import { tryLock } from ${JSON.stringify(pathToFileURL(GOALCTL).href)};
console.log(tryLock(process.argv[2]!, { command: 'holder' }) ? 'held' : 'busy');
setInterval(() => {}, 1000);
`);
    const child = spawn(process.execPath, [holder, path], { stdio: ['ignore', 'pipe', 'inherit'], windowsHide: true });
    spawned.push(child.pid!);
    const first = await new Promise<string>(done => child.stdout!.once('data', data => done(String(data).trim())));
    expect(first).toBe('held');
    expect(tryLock(path)).toBeUndefined();
    expect(lockHolder(path)?.pid).toBe(child.pid!);
    killTree(child.pid!);
    for (let i = 0; i < 50 && processInfo(child.pid!).alive; i++) await Bun.sleep(100);
    const release = tryLock(path);
    expect(release).toBeDefined();
    release!();
  }, 20_000);
});

// Temporary repositories: every git test runs in one, never in the Narrata checkout.
function gitIn(dir: string, ...args: string[]): string {
  const result = spawnSync('git', ['-c', 'core.quotePath=false', ...args], { cwd: dir, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(`git ${args.join(' ')}: ${result.stderr}`);
  return result.stdout.trim();
}

function write(dir: string, path: string, content: string): void {
  mkdirSync(join(dir, path, '..'), { recursive: true });
  writeFileSync(join(dir, path), content);
}

function repo(files: Record<string, string>): string {
  const dir = tempDir('repo');
  gitIn(dir, 'init', '-q', '-b', 'main');
  gitIn(dir, 'config', 'user.email', 'goal@example.invalid');
  gitIn(dir, 'config', 'user.name', 'goalctl test');
  gitIn(dir, 'config', 'commit.gpgsign', 'false');
  gitIn(dir, 'config', 'core.autocrlf', 'false');
  write(dir, '.gitignore', '.temp/\n/target/\n');
  for (const [path, content] of Object.entries(files)) write(dir, path, content);
  gitIn(dir, 'add', '.');
  gitIn(dir, 'commit', '-q', '-m', 'start');
  return dir;
}

describe('git helpers', () => {
  test('commits only the archived and removed paths and leaves what peers staged', () => {
    const dir = repo({ 'docs/goals/g/GOAL.md': '# g\n', 'docs/goals/g/tasks/G-001.md': 'brief\n' });
    write(dir, 'peer.rs', 'staged by a peer');
    gitIn(dir, 'add', 'peer.rs');
    commitPaths(dir, [{ path: 'archive/goals/g-2026-10-05/tasks/G-001.md', content: 'brief\n' }],
      ['docs/goals/g/tasks/G-001.md'], `Archive the closed brief G-001\n\n${commitTrailers(['g'])}`);
    expect(existsSync(join(dir, 'docs/goals/g/tasks'))).toBe(false);
    expect(existsSync(join(dir, 'docs/goals/g/GOAL.md'))).toBe(true);
    expect(gitIn(dir, 'show', '--no-renames', '--name-status', '--format=%s', 'HEAD').split('\n')).toEqual(['Archive the closed brief G-001', '',
      'A\tarchive/goals/g-2026-10-05/tasks/G-001.md', 'D\tdocs/goals/g/tasks/G-001.md']);
    expect(gitIn(dir, 'log', '-1', '--format=%(trailers:key=Goal,valueonly)%(trailers:key=Co-Authored-By,valueonly)'))
      .toBe('g\nClaude Opus 5.5 <noreply@anthropic.com>');
    expect(gitIn(dir, 'diff', '--cached', '--name-only')).toBe('peer.rs');
  });

  test('finds citations of a brief outside the excluded paths', () => {
    const dir = repo({ 'docs/goals/g/tasks/G-001.md': 'brief', 'crates/a/src/lib.rs': '// cites docs/goals/g/tasks/G-002.md\n',
      'archive/goals/g-2026-10-01/handoffs/G-001.md': 'G-001 docs/goals/g/tasks/G-002.md' });
    expect(treeMentions(dir, ['docs/goals/g/tasks/G-002.md'], { exclude: ['docs/goals/**', 'archive/**'] }))
      .toEqual(['crates/a/src/lib.rs:1:// cites docs/goals/g/tasks/G-002.md']);
    expect(treeMentions(dir, ['G-001'], { words: true })).toEqual([]);
    expect(treeMentions(dir, [])).toEqual([]);
  });

  test('lists Markdown commits without a Goal trailer as maintainer edits', () => {
    const dir = repo({ 'AGENTS.md': 'a\n' });
    const start = gitIn(dir, 'rev-parse', 'HEAD');
    write(dir, 'docs/goals/worker.md', 'maintainer edit\n');
    gitIn(dir, 'add', '.');
    gitIn(dir, 'commit', '-q', '-m', 'Clarify the handoff');
    write(dir, 'docs/goals/g/state.md', 'checkpoint\n');
    gitIn(dir, 'add', '.');
    gitIn(dir, 'commit', '-q', '-m', 'Checkpoint\n\nGoal: g');
    write(dir, 'src.rs', 'code\n');
    gitIn(dir, 'add', '.');
    gitIn(dir, 'commit', '-q', '-m', 'Code only');
    const edits = maintainerDocCommits(dir, start);
    expect(edits.map(edit => [edit.subject, edit.files])).toEqual([['Clarify the handoff', ['docs/goals/worker.md']]]);
  });

  test('rewrites relative links for a moved Markdown file and follows moved targets', () => {
    const brief = 'docs/goals/g/tasks/G-001.md';
    const archived = 'archive/goals/g-2026-10-05/tasks/G-001.md';
    const content = [
      '见 [Goal](../GOAL.md)、[工具链](../../../development/toolchain.md#bun) 与 ![图](../img/a%20b.png)。',
      '保留 `[code](../GOAL.md)`、[外链](https://example.com/x.md)、[本页](#结果) 和 [锚点](#bun)。',
      '[ref]: ../state.md "State"',
      '```md',
      '[fenced](../GOAL.md)',
      '```',
      '[行号](../../../../crates/a/src/lib.rs:42) [越界](../../../../../outside.md)\r',
    ].join('\n');
    const moved = relinkMarkdown(content, brief, archived);
    expect(moved.split('\n')).toEqual([
      '见 [Goal](../../../../docs/goals/g/GOAL.md)、[工具链](../../../../docs/development/toolchain.md#bun) 与 ![图](../../../../docs/goals/g/img/a%20b.png)。',
      '保留 `[code](../GOAL.md)`、[外链](https://example.com/x.md)、[本页](#结果) 和 [锚点](#bun)。',
      '[ref]: ../../../../docs/goals/g/state.md "State"',
      '```md',
      '[fenced](../GOAL.md)',
      '```',
      '[行号](../../../../crates/a/src/lib.rs:42) [越界](../../../../../outside.md)\r',
    ]);
    // When the Goal closes its directory follows, and the archived brief points back at the archived GOAL.md.
    const goalMove = (path: string) => path === 'docs/goals/g' || path.startsWith('docs/goals/g/')
      ? `archive/goals/g-2026-10-05${path.slice('docs/goals/g'.length)}` : undefined;
    expect(relinkMarkdown(moved, archived, archived, goalMove).split('\n')[0])
      .toBe('见 [Goal](../GOAL.md)、[工具链](../../../../docs/development/toolchain.md#bun) 与 ![图](../img/a%20b.png)。');
    expect(relinkMarkdown('[状态](state.md) [任务](tasks/)', 'docs/goals/g/GOAL.md', 'archive/goals/g-2026-10-05/GOAL.md', goalMove))
      .toBe('[状态](state.md) [任务](tasks)');
    expect(relinkMarkdown('[a](b.md)', 'x/y.md', 'x/y.md')).toBe('[a](b.md)');
  });

  test('parses porcelain -z with renames and non-ASCII names', () => {
    expect(parsePorcelainZ(' M crates/a.rs\0R  docs/新.md\0docs/old.md\0?? archive/叙事.md\0')).toEqual(['crates/a.rs', 'docs/新.md', 'archive/叙事.md']);
  });

  test('keeps a removed worktree\'s .temp artifacts beside the task run records', () => {
    const base = tempDir('preserve');
    const worktree = join(base, 'wt');
    const runDir = join(base, 'runs', 'G-001');
    write(worktree, '.temp/trace/replay.json', '{}');
    const first = preserveWorktreeArtifacts(worktree, runDir)!;
    expect(readFileSync(join(first, 'trace', 'replay.json'), 'utf8')).toBe('{}');
    expect(existsSync(join(worktree, '.temp'))).toBe(false);
    mkdirSync(join(worktree, '.temp'));
    expect(preserveWorktreeArtifacts(worktree, runDir)).not.toBe(first);
    expect(preserveWorktreeArtifacts(worktree, runDir)).toBeNull();
  });
});

// The whole lifecycle through the command line, with the fake CLI as worker, in a temporary repository.
function goalctl(dir: string, args: string[], env: Record<string, string> = {}): { code: number; out: string } {
  const base = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^GOAL_|^FAKE_/.test(key)));
  const result = spawnSync(process.execPath, [GOALCTL, ...args], { cwd: dir, encoding: 'utf8', windowsHide: true,
    env: { ...base, GOAL_CLAUDE_COMMAND: JSON.stringify([process.execPath, FAKE_CLI]), GOAL_POLL_MS: '100', ...env } });
  return { code: result.status ?? 1, out: `${result.stdout}${result.stderr}` };
}

function ok(dir: string, args: string[], env: Record<string, string> = {}): string {
  const result = goalctl(dir, args, env);
  if (result.code !== 0) throw new Error(`goalctl ${args.join(' ')} exited ${result.code}:\n${result.out}`);
  return result.out;
}

const goalText = (areas: string[]) => `---\n# 其他 Goal 不得认领\nareas:\n${areas.map(area => `  - ${area}`).join('\n')}\n---\n\n# Goal\n`;
const briefText = (id: string, paths: string[], extra = '') =>
  `---\nid: ${id}\ntitle: Task ${id}\nengine: claude\neffort: high\ncases: []\npaths: [${paths.join(', ')}]\nshared: []\ndepends: []\n${extra}---\n\n## 结果\n`;
/** The Goal's archive directory, named by its local start date (bun test itself runs in UTC). */
function archiveOf(dir: string): string {
  const archive = gitIn(dir, 'ls-tree', '--name-only', 'HEAD', 'archive/goals/');
  expect(archive).toMatch(/^archive\/goals\/alpha-\d{4}-\d{2}-\d{2}$/);
  return archive;
}

const ledgerOf = (dir: string) => JSON.parse(readFileSync(join(dir, '.temp/goal/ledger.json'), 'utf8')) as { tasks: Record<string, Task> };

function program(): string {
  return repo({
    'AGENTS.md': '# Agents\n', 'docs/goals/worker.md': '# Worker\n',
    'GOAL.md': '| [Alpha](docs/goals/alpha/GOAL.md) | running |\n',
    'docs/goals/alpha/GOAL.md': goalText(['crates/alpha/**']), 'docs/goals/alpha/state.md': '# State\n',
    'docs/goals/beta/GOAL.md': goalText(['crates/beta/**']),
    'crates/alpha/src/lib.rs': 'pub fn alpha() {}\n', 'crates/shared/src/lib.rs': 'pub fn shared() {}\n',
    'docs/development/toolchain.md': '# Toolchain\n\n## Bun\n',
  });
}

describe('lifecycle', () => {
  test('dispatch resolves repository and brief aliases before checking ownership', () => {
    const dir = program();
    const alias = join(tempDir('alias'), 'repository');
    symlinkSync(dir, alias, windows ? 'junction' : 'dir');
    ok(dir, ['goal', 'start', 'alpha', '--manager', 'a']);
    const path = 'docs/goals/alpha/tasks/G-001.md';
    write(dir, path, briefText('G-001', ['crates/alpha/**']));
    // Git and the caller can use different names for the same directory (including Windows 8.3 names).
    for (const { cwd, brief } of [{ cwd: dir, brief: join(alias, path) }, { cwd: alias, brief: join(dir, path) }]) {
      expect(ok(cwd, ['dispatch', brief, '--dry-run'])).toContain('G-001: claims ok');
      const refused = goalctl(cwd, ['dispatch', brief, '--dry-run'], { GOAL_ID: 'beta' });
      expect(refused.code).toBe(1);
      expect(refused.out).toContain('docs/goals/alpha/tasks/G-001.md belongs to Goal alpha, not beta');
    }
  }, 60_000);

  test('dispatch, wait, merge and close archive the brief and handoff in one commit', () => {
    const dir = program();
    ok(dir, ['goal', 'start', 'alpha', '--manager', 'alpha-manager']);
    expect(ok(dir, ['new', '--goal', 'alpha', 'Alpha', 'parser'])).toContain('G-001 reserved for Goal alpha');
    expect(parseBrief(readFileSync(join(dir, 'docs/goals/alpha/tasks/G-001.md'), 'utf8'))).toMatchObject({ id: 'G-001', engine: 'claude' });
    write(dir, 'docs/goals/alpha/tasks/G-001.md', `${briefText('G-001', ['crates/alpha/**'])}`
      + '见 [Goal](../GOAL.md) 与 [工具链](../../../development/toolchain.md#bun)。\n');
    write(dir, 'docs/goals/alpha/state.md', '# State\n\n- [G-001](tasks/G-001.md) 进行中\n');
    gitIn(dir, 'add', '.');
    gitIn(dir, 'commit', '-q', '-m', 'Brief G-001\n\nGoal: alpha');

    const dispatched = ok(dir, ['dispatch', 'docs/goals/alpha/tasks/G-001.md'], { FAKE_WRITE: 'crates/alpha/src/parser.rs',
      CLAUDECODE: '1', CLAUDE_CODE_SESSION_ID: 'manager-session', FAKE_NOTE: 'See [the brief](../GOAL.md) and ```` fences' });
    expect(dispatched).toContain('G-001 started: claude-opus-5-5/high');
    const worktree = join(dir, '.temp/worktrees/g-001');
    expect(readFileSync(join(worktree, '.temp/goal/brief.md'), 'utf8')).toContain('id: G-001');

    const waited = ok(dir, ['wait', 'G-001']);
    expect(waited).toContain('scope: ok');
    expect(waited).toContain('PROMPT names AGENTS.md, worker.md and the brief');
    expect(waited).toContain('CLAUDECODE unset');
    expect(waited).toContain('MANAGER alpha-manager');
    expect(waited).toMatch(/ARGS -p --model claude-opus-5-5 --effort high --dangerously-skip-permissions --session-id [0-9a-f-]{36} -n g-001/);
    expect(waited).toContain('cost $0.25');
    expect(existsSync(join(dir, '.temp/goal/handoffs/G-001.md'))).toBe(true);
    expect(ledgerOf(dir).tasks['G-001']!.state).toBe('exited');
    expect(waited).toContain('exit code 0');
    expect(JSON.parse(ok(dir, ['usage', 'G-001'])).totals.claude).toMatchObject({ attempts: 1, cost: 0.25, output: 20 });

    // Resume continues the same session with only the manager's message; --fresh starts over with the full prompt.
    const sessionId = /--session-id ([0-9a-f-]{36})/.exec(waited)![1]!;
    expect(ok(dir, ['resume', 'G-001', '-m', 'Add the parser tests', '--effort', 'max'])).toContain('resumed');
    const resumed = ok(dir, ['wait', 'G-001']);
    expect(resumed).toContain(`--effort max --dangerously-skip-permissions --resume ${sessionId}`);
    expect(resumed).toContain('PROMPT INCOMPLETE');
    expect(ok(dir, ['resume', 'G-001', '-m', 'Start over', '--fresh'])).toContain('fresh session');
    expect(ok(dir, ['wait', 'G-001'])).toContain('PROMPT names AGENTS.md, worker.md and the brief');
    expect(readFileSync(join(dir, '.temp/goal/runs/G-001/attempt-3.prompt.md'), 'utf8')).toContain('Manager note:\nStart over');

    expect(ok(dir, ['merge', 'G-001'])).toContain('G-001 merged');
    expect(readFileSync(join(dir, 'crates/alpha/src/parser.rs'), 'utf8')).toContain('fake worker wrote');

    write(dir, 'peer.rs', 'staged by a peer');
    gitIn(dir, 'add', 'peer.rs');
    const closed = ok(dir, ['close', 'G-001', 'verified']);
    expect(closed).toContain('Archive the closed brief G-001');
    const archive = archiveOf(dir);
    expect(gitIn(dir, 'show', '--no-renames', '--name-only', '--format=%s%n%b', 'HEAD').split('\n')).toEqual(expect.arrayContaining([
      'Archive the closed brief G-001', 'Goal: alpha', 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>',
      `${archive}/handoffs/G-001.md`, `${archive}/tasks/G-001.md`, 'docs/goals/alpha/tasks/G-001.md']));
    expect(readFileSync(join(dir, archive, 'handoffs/G-001.md'), 'utf8')).toContain('`````text\nRESULT: done');
    expect(existsSync(join(dir, 'docs/goals/alpha/tasks/G-001.md'))).toBe(false);
    // Links still resolve after the move: the brief's own, the Goal checkpoint's link to it and the handoff (fenced).
    expect(readFileSync(join(dir, archive, 'tasks/G-001.md'), 'utf8')).toContain('[Goal](../../../../docs/goals/alpha/GOAL.md)');
    expect(readFileSync(join(dir, 'docs/goals/alpha/state.md'), 'utf8')).toContain(`[G-001](../../../${archive}/tasks/G-001.md)`);
    expect(gitIn(dir, 'show', '--name-only', '--format=', 'HEAD')).toContain('docs/goals/alpha/state.md');
    for (const file of [`${archive}/tasks/G-001.md`, `${archive}/handoffs/G-001.md`, 'docs/goals/alpha/state.md']) {
      expect(checkFile(join(dir, file))).toEqual([]);
    }
    expect(gitIn(dir, 'diff', '--cached', '--name-only')).toBe('peer.rs');
    expect(existsSync(worktree)).toBe(false);
    expect(gitIn(dir, 'branch', '--list', 'goal/g-001')).toBe('');
    expect(ledgerOf(dir).tasks['G-001']!.state).toBe('verified');
    expect(ok(dir, ['new', '--goal', 'alpha', 'Next'])).toContain('G-002 reserved');
  }, 120_000);

  test('refuses overlapping Goals, other Goals\' areas, held claims and signed-out engines', () => {
    const dir = program();
    write(dir, 'docs/goals/gamma/GOAL.md', goalText(['crates/**']));
    ok(dir, ['goal', 'start', 'alpha', '--manager', 'a']);
    ok(dir, ['goal', 'start', 'beta', '--manager', 'b']);
    expect(goalctl(dir, ['goal', 'start', 'gamma', '--manager', 'c']).out).toContain('areas overlap another Goal\'s');

    write(dir, 'docs/goals/alpha/tasks/G-001.md', briefText('G-001', ['crates/beta/src/**']));
    expect(goalctl(dir, ['dispatch', 'docs/goals/alpha/tasks/G-001.md', '--dry-run']).out)
      .toContain('lies in Goal beta\'s area crates/beta/**');
    write(dir, 'docs/goals/alpha/tasks/G-002.md', briefText('G-001', ['crates/alpha/**']));
    expect(goalctl(dir, ['dispatch', 'docs/goals/alpha/tasks/G-002.md']).out).toContain('the file name and id must match');
    write(dir, 'docs/goals/alpha/tasks/G-003.md', briefText('G-003', ['crates/alpha/**'], 'migrations: [a:001-002]\n'));
    expect(goalctl(dir, ['dispatch', 'docs/goals/alpha/tasks/G-003.md']).out).toContain('no numbered migrations');

    write(dir, 'docs/goals/alpha/tasks/G-004.md', briefText('G-004', ['crates/alpha/**']));
    expect(goalctl(dir, ['dispatch', 'docs/goals/alpha/tasks/G-004.md'], { GOAL_ID: 'beta' }).out).toContain('belongs to Goal alpha');
    const refused = goalctl(dir, ['dispatch', 'docs/goals/alpha/tasks/G-004.md'], { FAKE_LOGGED_IN: '0' });
    expect(refused.code).toBe(1);
    expect(refused.out).toContain('claude is not signed in');
    expect(refused.out).toContain('claude auth login');
    expect(existsSync(join(dir, '.temp/worktrees/g-004'))).toBe(false);

    ok(dir, ['dispatch', 'docs/goals/alpha/tasks/G-004.md'], { FAKE_WRITE: 'crates/alpha/src/a.rs' });
    write(dir, 'docs/goals/alpha/tasks/G-005.md', briefText('G-005', ['crates/alpha/src/*.rs']));
    expect(goalctl(dir, ['dispatch', 'docs/goals/alpha/tasks/G-005.md', '--dry-run']).out).toContain('overlaps G-004 crates/alpha/**');
    ok(dir, ['wait', 'G-004']);
    expect(goalctl(dir, ['owner', 'crates/alpha/src/a.rs']).out).toContain('claimed by G-004 (exited)');
    expect(goalctl(dir, ['owner', 'crates/beta/x.rs']).out).toContain('unclaimed; in Goal beta\'s area');
    expect(goalctl(dir, ['merge', 'G-004'], { GOAL_ID: 'beta' }).out).toContain('belongs to Goal alpha');
  }, 120_000);

  test('merge refuses out-of-claim and task-named files and marks a rebase conflict', () => {
    const dir = program();
    ok(dir, ['goal', 'start', 'alpha', '--manager', 'a']);
    write(dir, 'docs/goals/alpha/tasks/G-001.md', briefText('G-001', ['crates/alpha/**']));
    write(dir, 'docs/goals/alpha/tasks/G-002.md', briefText('G-002', ['crates/history/**']));
    write(dir, 'docs/goals/alpha/tasks/G-003.md', briefText('G-003', ['crates/conflict/**']));
    ok(dir, ['dispatch', 'docs/goals/alpha/tasks/G-001.md'], { FAKE_WRITE: 'crates/alpha/src/a.rs,crates/shared/src/lib.rs' });
    ok(dir, ['dispatch', 'docs/goals/alpha/tasks/G-002.md'], { FAKE_WRITE: 'crates/history/tests/g-002-replay.rs' });
    ok(dir, ['dispatch', 'docs/goals/alpha/tasks/G-003.md'], { FAKE_WRITE: 'crates/conflict/src/lib.rs', FAKE_CONTENT: 'worker' });
    for (const id of ['G-001', 'G-002', 'G-003']) ok(dir, ['wait', id]);

    expect(goalctl(dir, ['scope', 'G-001']).out).toContain('scope: VIOLATIONS\n  crates/shared/src/lib.rs');
    const outside = goalctl(dir, ['merge', 'G-001']);
    expect(outside.out).toContain('changed files outside its claim:\n  crates/shared/src/lib.rs');
    expect(ledgerOf(dir).tasks['G-001']!.state).toBe('exited');
    expect(ok(dir, ['merge', 'G-001', '--allow-scope'])).toContain('G-001 merged');

    expect(goalctl(dir, ['merge', 'G-002']).out).toContain('crates/history/tests/g-002-replay.rs: named after a task');
    expect(ok(dir, ['merge', 'G-002', '--allow-ids'])).toContain('G-002 merged');

    write(dir, 'crates/conflict/src/lib.rs', 'main\n');
    gitIn(dir, 'add', '.');
    gitIn(dir, 'commit', '-q', '-m', 'Main changes the same file');
    const conflict = goalctl(dir, ['merge', 'G-003']);
    expect(conflict.code).toBe(1);
    expect(conflict.out).toContain('marked conflict');
    expect(conflict.out).toContain('crates/conflict/src/lib.rs');
    expect(ledgerOf(dir).tasks['G-003']!.state).toBe('conflict');
    expect(gitIn(join(dir, '.temp/worktrees/g-003'), 'status', '--porcelain')).toBe('');
    expect(goalctl(dir, ['close', 'G-003', 'verified']).out).toContain('G-003 is conflict, not merged');
  }, 120_000);

  test('stop ends a detached worker and its descendants; close cancelled removes the worktree', async () => {
    const dir = program();
    ok(dir, ['goal', 'start', 'alpha', '--manager', 'a']);
    write(dir, 'docs/goals/alpha/tasks/G-001.md', briefText('G-001', ['crates/alpha/**']));
    ok(dir, ['dispatch', 'docs/goals/alpha/tasks/G-001.md'], { FAKE_MODE: 'sleep' });
    const pidsFile = join(dir, '.temp/worktrees/g-001/.temp/fake-pids.json');
    for (let i = 0; i < 100 && !existsSync(pidsFile); i++) await Bun.sleep(100);
    const pids = JSON.parse(readFileSync(pidsFile, 'utf8')) as { worker: number; child: number };
    spawned.push(pids.worker, pids.child);
    // The dispatching process has exited; the worker lives on.
    expect(processInfo(pids.worker).alive).toBe(true);
    expect(ok(dir, ['status'])).toMatch(/G-001 running/);
    expect(goalctl(dir, ['resume', 'G-001', '-m', 'more']).out).toContain('stop it first');
    expect(ok(dir, ['stop', 'G-001'])).toContain('G-001 stopped');
    for (let i = 0; i < 50 && processInfo(pids.child).alive; i++) await Bun.sleep(100);
    expect(processInfo(pids.worker).alive).toBe(false);
    expect(processInfo(pids.child).alive).toBe(false);
    expect(ledgerOf(dir).tasks['G-001']!.state).toBe('stopped');
    expect(ok(dir, ['close', 'G-001', 'cancelled'])).toContain('G-001 cancelled');
    expect(existsSync(join(dir, '.temp/worktrees/g-001'))).toBe(false);
    expect(existsSync(join(dir, '.temp/goal/runs/G-001/worktree-temp/fake-pids.json'))).toBe(true);
    expect(gitIn(dir, 'branch', '--list', 'goal/g-001')).toContain('goal/g-001');
  }, 120_000);

  test('goal close waits for convergence, then moves the Goal directory to the archive', () => {
    const dir = program();
    ok(dir, ['goal', 'start', 'alpha', '--manager', 'a']);
    write(dir, 'docs/goals/alpha/GOAL.md', `${goalText(['crates/alpha/**'])}见 [状态](state.md) 与 [工具链](../../development/toolchain.md#bun)。\n`);
    write(dir, 'docs/goals/alpha/tasks/G-001.md', `${briefText('G-001', ['crates/alpha/**'])}见 [Goal](../GOAL.md)。\n`);
    gitIn(dir, 'add', '.');
    gitIn(dir, 'commit', '-q', '-m', 'Brief\n\nGoal: alpha');
    ok(dir, ['dispatch', 'docs/goals/alpha/tasks/G-001.md'], { FAKE_WRITE: 'crates/alpha/src/a.rs' });
    ok(dir, ['wait', 'G-001']);
    expect(goalctl(dir, ['goal', 'close', 'alpha', '--dry-run']).out).toContain('G-001 is exited');
    ok(dir, ['merge', 'G-001']);
    ok(dir, ['close', 'G-001', 'verified']);
    const pending = goalctl(dir, ['goal', 'close', 'alpha', '--dry-run']);
    expect(pending.code).toBe(1);
    expect(pending.out).toContain('links into the Goal: GOAL.md:1:');
    write(dir, 'GOAL.md', '| Goal | Outcome |\n');
    gitIn(dir, 'commit', '-q', '-am', 'Remove the Alpha row\n\nGoal: alpha');
    expect(ok(dir, ['goal', 'close', 'alpha', '--dry-run'])).toContain('has converged');
    expect(ok(dir, ['goal', 'close', 'alpha'])).toContain('Goal alpha closed');
    const archive = archiveOf(dir);
    expect(gitIn(dir, 'ls-tree', '-r', '--name-only', 'HEAD', archive).split('\n')).toEqual([`${archive}/GOAL.md`,
      `${archive}/handoffs/G-001.md`, `${archive}/ledger.json`, `${archive}/state.md`, `${archive}/tasks/G-001.md`]);
    expect(existsSync(join(dir, 'docs/goals/alpha'))).toBe(false);
    expect(readFileSync(join(dir, archive, 'GOAL.md'), 'utf8')).toContain('[状态](state.md) 与 [工具链](../../../docs/development/toolchain.md#bun)');
    expect(readFileSync(join(dir, archive, 'tasks/G-001.md'), 'utf8')).toContain('[Goal](../GOAL.md)');
    for (const file of ['GOAL.md', 'state.md', 'tasks/G-001.md', 'handoffs/G-001.md']) expect(checkFile(join(dir, archive, file))).toEqual([]);
    expect(gitIn(dir, 'log', '-1', '--format=%s%n%b')).toContain('Goal: alpha');
    expect(ok(dir, ['goal', 'start', 'beta', '--manager', 'b'])).toContain('Goal beta started');
    expect(ok(dir, ['new', '--goal', 'beta', 'After alpha'])).toContain('G-002 reserved');
  }, 120_000);
});

describe('check slots', () => {
  test('a slot runs the command and frees itself; heavy commands also hold the heavy lock', () => {
    const dir = repo({ 'scripts/check-x.ps1': 'if (Test-Path .temp/goal/slots/heavy/owner.json) { exit 9 } else { exit 4 }\n' });
    const count = "const fs = require('fs'); const d = '.temp/goal/slots'; process.exit(fs.readdirSync(d).filter(n => /^light-\\d+$/.test(n)).length * 10 + (fs.existsSync(d + '/heavy/owner.json') ? 1 : 0))";
    expect(goalctl(dir, ['slot', '--', process.execPath, '-e', count]).code).toBe(10);
    expect(goalctl(dir, ['slot', '--heavy', '--', process.execPath, '-e', count]).code).toBe(11);
    // A command inside a slot takes no second one.
    expect(goalctl(dir, ['slot', '--', process.execPath, GOALCTL, 'slot', '--', process.execPath, '-e', count]).code).toBe(10);
    expect(goalctl(dir, ['slot', '--', 'scripts/check-x.ps1']).code).toBe(9);
    expect(goalctl(dir, ['slot', '--', 'goalctl-no-such-command']).code).not.toBe(0);
    expect(spawnSync(process.execPath, ['-e', "process.exit(require('fs').readdirSync('.temp/goal/slots').length)"], { cwd: dir }).status).toBe(0);
  }, 60_000);
});
