import { describe, expect, test } from 'bun:test';
import { anchorsOf, linksOf, slugify, stripCode } from './check.ts';

describe('slugify', () => {
  test('matches GitHub slugs for mixed-language headings', () => {
    expect(slugify('Briefs and duplicate prevention')).toBe('briefs-and-duplicate-prevention');
    expect(slugify('三、照搬 manager–goal 的做法')).toBe('三照搬-managergoal-的做法');
    expect(slugify('`goalctl` 命令')).toBe('goalctl-命令');
  });
});

describe('anchorsOf', () => {
  test('numbers duplicate headings and ignores fenced code', () => {
    const anchors = anchorsOf('# A\n\n## A\n\n```md\n# Hidden\n```\n');
    expect([...anchors]).toEqual(['a', 'a-1']);
  });
});

describe('linksOf', () => {
  test('finds inline and reference links outside code', () => {
    const links = linksOf('See [x](a.md#b) and `[y](skip.md)`.\n\n[ref]: ../c.md\n```\n[z](no.md)\n```');
    expect(links).toEqual([
      { target: 'a.md#b', line: 1 },
      { target: '../c.md', line: 3 },
    ]);
  });

  test('keeps line numbers when code is stripped', () => {
    expect(stripCode('a\n```\nb\n```\nc').split('\n')).toHaveLength(5);
  });
});
