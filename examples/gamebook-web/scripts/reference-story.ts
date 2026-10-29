import type { ContentOutline, OptionSource, PackageSource, ProjectManifest } from "@rezics/narrata/authoring";

/** Compound acceptance work, separate from the minimal shortStorySource sample. No body text. */
export function referenceStorySource() {
  const ref = (key: string) => ({ provider: "signal", key });
  const read = (scope: "shared" | "parameter", name: string) => ({ kind: "read" as const, scope, name });
  const branch = (key: string, target: string): OptionSource => ({ key, label: ref(`option.${key}`), outcome: { kind: "branch", target } });
  const passage = (key: string, data: Record<string, unknown>) => ({ type_id: "narrata.passage", data: { title: ref(`scene.${key}`), body: { unit: ref(key) }, ...data } });
  const manifest: ProjectManifest = { format_version: 2, packages: { signal: "signal.json" }, product: {
    id: "signal-lantern", title: ref("title"), entry: { package: "signal", graph: "journey" }, shared: { signal: false },
    endings: { home: { title: ref("ending.home"), body: { unit: ref("home") } }, away: { title: ref("ending.away"), body: { unit: ref("away") } } },
  } };
  const pkg: PackageSource = { id: "example.signal-lantern", version: "1.0.0", exports: ["journey"], graphs: {
    journey: { entry: "station", shared: { signal: "bool" }, outcomes: ["away", "home"], nodes: {
      station: passage("station", { body: { unit: ref("station"), first: "arrival", last: "departure" }, choice_points: [
        { key: "listen", placement: "arrival", options: [
          { key: "take", label: ref("option.take"), effects: [{ target: { scope: "shared", name: "signal" }, value: { kind: "literal", value: true } }],
            outcome: { kind: "local", reply: { unit: ref("station"), first: "accepted", last: "accepted" }, rejoin: "rejoin" } },
          { key: "leave", label: ref("option.leave"), outcome: { kind: "local", reply: { unit: ref("station"), first: "declined", last: "declined" }, rejoin: "rejoin" } },
        ] },
        { key: "route", placement: "departure", options: [branch("visit", "visit"), branch("river", "river")] },
      ] }),
      visit: { type_id: "narrata.call", data: { target: { kind: "local", graph: "keeper" }, arguments: { pass: { kind: "literal", value: 3 } }, on_return: { back: "dawn" } } },
      dawn: passage("dawn", { args: { signal: read("shared", "signal") }, choice_points: [{ key: "finish", options: [
        { ...branch("home", "home"), enabled_if: read("shared", "signal"), reason: ref("needSignal") }, branch("away", "away"), branch("linger", "river"),
      ] }] }),
      river: passage("river", { next: "away" }),
      home: { type_id: "narrata.return", data: { outcome: "home" } },
      away: { type_id: "narrata.return", data: { outcome: "away" } },
    } },
    keeper: { entry: "keeper", parameters: { pass: "int" }, outcomes: ["back"], nodes: {
      keeper: passage("keeper", { args: { pass: read("parameter", "pass") }, choice_points: [{ key: "return", options: [branch("return", "back")] }] }),
      back: { type_id: "narrata.return", data: { outcome: "back" } },
    } },
  } };
  return { manifest, packages: [["signal", pkg]] as const };
}

export function referenceStoryOutline(): ContentOutline {
  return { format_version: 1, provider: "signal", units: {
    station: { blocks: ["arrival", "accepted", "declined", "rejoin", "departure"] },
    keeper: { blocks: ["p1"] }, dawn: { blocks: ["p1"] }, river: { blocks: ["p1"] }, home: { blocks: ["p1"] }, away: { blocks: ["p1"] },
  } };
}
