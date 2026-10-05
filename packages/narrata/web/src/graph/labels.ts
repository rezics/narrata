export interface LabelBox { readonly left: number; readonly top: number; readonly right: number; readonly bottom: number }

/** Canvas abbreviations keep full identities in the host's accessible node list. */
export function graphCanvasLabel(label: string, id: string): string {
  return label === `〔${id}〕` ? `〔${id.slice(0, 4)}…${id.slice(-4)}〕` : label;
}

/** A small view may project distinct published endpoints into the same pixel. */
export function projectionGroups(points: readonly { readonly id: string; readonly x: number; readonly y: number }[]): ReadonlyMap<string, { readonly key: string; readonly count: number }> {
  const parents = points.map((_, index) => index);
  const root = (index: number): number => { while (parents[index] !== index) index = parents[index]!; return index; };
  for (let a = 0; a < points.length; a++) for (let b = a + 1; b < points.length; b++) {
    const dx = points[a]!.x - points[b]!.x, dy = points[a]!.y - points[b]!.y;
    if (dx * dx + dy * dy <= 1) parents[root(b)] = root(a);
  }
  const counts = new Map<number, number>();
  points.forEach((_, index) => { const group = root(index); counts.set(group, (counts.get(group) ?? 0) + 1); });
  return new Map(points.map((point, index) => { const group = root(index); return [point.id, Object.freeze({ key: points[group]!.id, count: counts.get(group)! })]; }));
}

/** Places annotations only; published node coordinates never change. */
export function createLabelPlacement(width: number, height: number) {
  const cells = new Map<string, LabelBox[]>();
  const cellSize = 64, margin = 8, gap = 4;
  const keys = (box: LabelBox) => {
    const result: string[] = [];
    for (let x = Math.floor(box.left / cellSize); x <= Math.floor(box.right / cellSize); x++)
      for (let y = Math.floor(box.top / cellSize); y <= Math.floor(box.bottom / cellSize); y++) result.push(`${x}:${y}`);
    return result;
  };
  return (x: number, y: number, radius: number, textWidth: number, textHeight: number): LabelBox | null => {
    const padding = radius + 6;
    const verticalPadding = Math.max(padding, textHeight / 2 + gap + 2);
    const candidates = [
      [x + padding, y - textHeight / 2], [x - padding - textWidth, y - textHeight / 2],
      [x - textWidth / 2, y + verticalPadding], [x - textWidth / 2, y - verticalPadding - textHeight],
    ];
    for (const [left, top] of candidates) {
      const box = { left: left!, top: top!, right: left! + textWidth, bottom: top! + textHeight };
      if (box.left < margin || box.top < margin || box.right > width - margin || box.bottom > height - margin) continue;
      const bucketKeys = keys(box);
      if (bucketKeys.some(key => cells.get(key)?.some(other => box.left < other.right + gap && box.right + gap > other.left && box.top < other.bottom + gap && box.bottom + gap > other.top))) continue;
      // Include the collision margin in the grid so adjacent cells cannot miss overlaps.
      for (const key of keys({ left: box.left - gap, top: box.top - gap, right: box.right + gap, bottom: box.bottom + gap })) {
        const bucket = cells.get(key) ?? []; bucket.push(box); cells.set(key, bucket);
      }
      return box;
    }
    return null;
  };
}
