// Checks that relative links and heading anchors in the repository's Markdown resolve.
// Usage: bun scripts/docs/check.ts [paths...]   (defaults to every tracked or untracked-but-unignored *.md)
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';

export type Problem = { file: string; line: number; message: string };

const ROOT = resolve(import.meta.dir, '..', '..');

/** GitHub-style heading slug: lower-case, drop punctuation, spaces become hyphens. */
export function slugify(heading: string): string {
  return heading
    .trim()
    .toLowerCase()
    .replace(/<[^>]*>/g, '')
    .replace(/`/g, '')
    .replace(/[^\p{L}\p{N}\p{M}\s_-]/gu, '')
    .replace(/\s/g, '-');
}

/** Removes fenced code blocks and inline code, keeping line numbers stable. */
export function stripCode(markdown: string): string {
  const lines = markdown.split(/\r?\n/);
  let fence: string | null = null;
  return lines
    .map((line) => {
      const marker = /^\s{0,3}(`{3,}|~{3,})/.exec(line)?.[1];
      if (fence) {
        if (marker && marker[0] === fence[0] && marker.length >= fence.length) fence = null;
        return '';
      }
      if (marker) {
        fence = marker;
        return '';
      }
      return line.replace(/(`+)[^`]*?\1/g, '');
    })
    .join('\n');
}

export function anchorsOf(markdown: string): Set<string> {
  const anchors = new Set<string>();
  const counts = new Map<string, number>();
  for (const line of stripCode(markdown).split('\n')) {
    const heading = /^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$/.exec(line)?.[1];
    if (heading === undefined) continue;
    const base = slugify(heading);
    const seen = counts.get(base) ?? 0;
    counts.set(base, seen + 1);
    anchors.add(seen === 0 ? base : `${base}-${seen}`);
  }
  for (const match of markdown.matchAll(/<a\s+(?:name|id)="([^"]+)"/g)) anchors.add(match[1]!);
  return anchors;
}

export function linksOf(markdown: string): { target: string; line: number }[] {
  const links: { target: string; line: number }[] = [];
  stripCode(markdown)
    .split('\n')
    .forEach((text, index) => {
      for (const match of text.matchAll(/!?\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)/g)) {
        links.push({ target: match[1]!, line: index + 1 });
      }
      const definition = /^\s{0,3}\[[^\]]+\]:\s*<?([^\s>]+)>?/.exec(text);
      if (definition) links.push({ target: definition[1]!, line: index + 1 });
    });
  return links;
}

const anchorCache = new Map<string, Set<string>>();
function anchorsFor(file: string): Set<string> {
  let anchors = anchorCache.get(file);
  if (!anchors) {
    anchors = anchorsOf(readFileSync(file, 'utf8'));
    anchorCache.set(file, anchors);
  }
  return anchors;
}

export function checkFile(file: string): Problem[] {
  const problems: Problem[] = [];
  const markdown = readFileSync(file, 'utf8');
  const shown = relative(ROOT, file).replaceAll('\\', '/');
  for (const { target, line } of linksOf(markdown)) {
    if (/^[a-z][a-z0-9+.-]*:/i.test(target) && !/^[a-z]:[\\/]/i.test(target)) continue; // http:, mailto:, ...
    const [rawPath, rawAnchor] = target.split('#', 2);
    let path = decodeURIComponent(rawPath ?? '');
    let destination = path === '' ? file : resolve(dirname(file), path);
    if (path !== '' && !existsSync(destination)) {
      const withoutLine = path.replace(/:\d+(?:-\d+)?$/, ''); // `file.rs:42` style references
      if (withoutLine !== path && existsSync(resolve(dirname(file), withoutLine))) {
        path = withoutLine;
        destination = resolve(dirname(file), withoutLine);
      } else {
        problems.push({ file: shown, line, message: `missing link target ${target}` });
        continue;
      }
    }
    if (rawAnchor === undefined || rawAnchor === '') continue;
    if (!destination.endsWith('.md') || statSync(destination).isDirectory()) continue;
    const anchor = decodeURIComponent(rawAnchor).toLowerCase();
    if (/^l\d+(-l\d+)?$/.test(anchor)) continue; // GitHub line anchors
    if (!anchorsFor(destination).has(anchor)) {
      problems.push({ file: shown, line, message: `missing anchor #${rawAnchor} in ${relative(ROOT, destination).replaceAll('\\', '/')}` });
    }
  }
  return problems;
}

export function markdownFiles(): string[] {
  const listed = spawnSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z', '--', '*.md'], {
    cwd: ROOT,
    encoding: 'utf8',
  });
  if (listed.status !== 0) throw new Error(`git ls-files failed: ${listed.stderr}`);
  return listed.stdout
    .split('\0')
    .filter((path) => path !== '' && !path.startsWith('.temp/'))
    .map((path) => join(ROOT, path))
    .filter((path) => existsSync(path));
}

if (import.meta.main) {
  const requested = process.argv.slice(2);
  const files = requested.length > 0 ? requested.map((path) => resolve(path)) : markdownFiles();
  const problems = files.flatMap(checkFile);
  for (const problem of problems) console.error(`${problem.file}:${problem.line}: ${problem.message}`);
  console.log(`docs check: ${files.length} files, ${problems.length} problems`);
  process.exit(problems.length === 0 ? 0 : 1);
}
