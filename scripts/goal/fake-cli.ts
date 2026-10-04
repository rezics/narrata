// Stand-in for the `claude` CLI in goalctl's tests (GOAL_CLAUDE_COMMAND): it answers the sign-in probe, reads the
// prompt from stdin, does the work scripted in its environment and prints Claude Code's JSON result. No model runs.
import { spawn, spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';

const args = process.argv.slice(2);
if (args[0] === 'auth') {
  const signedIn = process.env.FAKE_LOGGED_IN !== '0';
  console.log(JSON.stringify({ loggedIn: signedIn, authMethod: signedIn ? 'claude.ai' : 'none' }));
  process.exit(signedIn ? 0 : 1);
}
const prompt = await Bun.stdin.text();
const after = (flag: string): string | undefined => (args.includes(flag) ? args[args.indexOf(flag) + 1] : undefined);
const session = after('--session-id') ?? after('--resume') ?? '';

if (process.env.FAKE_MODE === 'sleep') {
  // A detached grandchild leaves this process's job object, so only a tree kill reaches it.
  const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { detached: true, stdio: 'ignore', windowsHide: true });
  mkdirSync('.temp', { recursive: true });
  writeFileSync('.temp/fake-pids.json', JSON.stringify({ worker: process.pid, child: child.pid }));
  setInterval(() => {}, 1000);
} else {
  const files = (process.env.FAKE_WRITE ?? '').split(',').filter(Boolean);
  for (const file of files) {
    mkdirSync(dirname(file), { recursive: true });
    // No task ID in the content: merge refuses files that start citing tasks.
    writeFileSync(file, `${process.env.FAKE_CONTENT ?? `fake worker wrote ${file}`}\n`);
  }
  if (files.length) {
    for (const args of [['add', '--', ...files], ['commit', '-q', '-m', `Work for ${process.env.GOAL_TASK_ID}\n\nGoal: ${process.env.GOAL_ID}`]]) {
      const git = spawnSync('git', args, { encoding: 'utf8' });
      if (git.status !== 0) console.error(`git ${args[0]} failed (${git.status} ${git.signal} ${git.error?.message}): ${git.stdout}${git.stderr}`);
    }
  }
  const reads = ['AGENTS.md', 'docs/goals/worker.md', '.temp/goal/brief.md'].every(path => prompt.includes(path));
  console.log(JSON.stringify({ type: 'result', is_error: false, session_id: session, total_cost_usd: 0.25,
    usage: { input_tokens: 10, cache_creation_input_tokens: 5, cache_read_input_tokens: 100, output_tokens: 20 },
    result: [`RESULT: done`, `PROMPT ${reads ? 'names AGENTS.md, worker.md and the brief' : 'INCOMPLETE'}`,
      `ARGS ${args.join(' ')}`, `CLAUDECODE ${process.env.CLAUDECODE ?? 'unset'}`, `MANAGER ${process.env.GOAL_MANAGER}`,
      ...process.env.FAKE_NOTE ? [process.env.FAKE_NOTE] : []].join('\n') }));
}
