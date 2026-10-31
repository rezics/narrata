import { openVn, type OpenVnOptions, type VnReader, type VnViewV1 } from "@rezics/narrata/vn";
import type { ExecutionId } from "@rezics/narrata/vn";

const execution: ExecutionId = "execution:0190f2a0000070008000000000000001";
const bytes = new Uint8Array();
const complete: OpenVnOptions = { pack: bytes, execution };
const lazy: OpenVnOptions = { manifest: bytes, execution, fetchChunk: async () => bytes };
const open: (options: OpenVnOptions) => Promise<VnReader> = openVn;

function act(reader: VnReader, view: VnViewV1): Promise<string | void> {
  const interaction = view.view.interaction;
  if (interaction.kind === "advance") return reader.advance(view.view.cursor, interaction.dialogue);
  if (interaction.kind === "choose") {
    const option = interaction.options.find(candidate => candidate.enabled);
    return option ? reader.choose(view.view.cursor, interaction.choice_point, [option.id]) : Promise.resolve();
  }
  return Promise.resolve();
}

declare const reader: VnReader;
// @ts-expect-error Advance accepts a dialogue identity, not choice options.
reader.advance("commit", "node", []);
// @ts-expect-error Choose requires the current choice point and selected option IDs.
reader.choose("commit", []);

void [complete, lazy, open, act];
