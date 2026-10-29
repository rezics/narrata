import type { ContentOutline, OptionSource, PackageSource, ProjectManifest } from "../generated/authoring.js";

export interface SampleSource {
  readonly manifest: ProjectManifest;
  readonly packages: readonly (readonly [string, PackageSource])[];
}

/**
 * The smallest work written with the public authoring types: three passage bodies, two
 * choice points and two endings. It has no IDs; an IdMinter fills them before splitting.
 */
export function shortStorySource(provider = "story"): SampleSource {
  const ref = (key: string) => ({ provider, key });
  const branch = (key: string, target: string): OptionSource => ({ key, label: ref(`option.${key}`), outcome: { kind: "branch", target } });
  const passage = (body: string, data: Record<string, unknown>) => ({ type_id: "narrata.passage", data: { body: { unit: ref(body) }, ...data } });
  const manifest: ProjectManifest = {
    format_version: 2,
    packages: { story: "packages/story.json" },
    product: {
      id: "short-story", title: ref("title"), entry: { package: "story", graph: "main" },
      endings: { home: { title: ref("ending.home") }, away: { title: ref("ending.away") } },
    },
  };
  const story: PackageSource = {
    id: "example.short-story", version: "1.0.0", exports: ["main"],
    graphs: { main: { outcomes: ["away", "home"], entry: "start", nodes: {
      start: passage("start", { choice_points: [{ key: "path", options: [branch("forest", "forest"), branch("river", "river")] }] }),
      forest: passage("forest", { choice_points: [{ key: "return", options: [branch("home", "home"), branch("wander", "away")] }] }),
      river: passage("river", { next: "away" }),
      home: { type_id: "narrata.return", data: { outcome: "home" } },
      away: { type_id: "narrata.return", data: { outcome: "away" } },
    } } },
  };
  return { manifest, packages: [["story", story]] };
}

/** The provider's outline for shortStorySource: one block per body unit, no text. */
export function shortStoryOutline(provider = "story"): ContentOutline {
  return { format_version: 1, provider, units: { start: { blocks: ["p1"] }, forest: { blocks: ["p1"] }, river: { blocks: ["p1"] } } };
}
