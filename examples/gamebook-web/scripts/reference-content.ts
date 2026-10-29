import type { Resolution } from "@rezics/narrata/reader";

const labels: Readonly<Record<string, string>> = {
  title: "The Signal Lantern", "scene.station": "The last train", "scene.keeper": "The keeper's desk", "scene.dawn": "At the morning gate", "scene.river": "The river path",
  "option.take": "Accept the signal lantern", "option.leave": "Leave the lantern on the bench", "option.visit": "Visit the keeper with your pass", "option.river": "Follow the river instead",
  "option.return": "Return to the morning gate", "option.home": "Take the marked road home", "option.away": "Set out for the coast", "option.linger": "Wait beside the river",
  "ending.home": "A light in the window", "ending.away": "The open sea", needSignal: "The road home needs the signal lantern.",
};
const bodies: Readonly<Record<string, readonly { id: string; text: string }[]>> = {
  station: [
    { id: "arrival", text: "The last train left Mara beside an empty platform. On the bench, a brass lantern pulsed with a small green light." },
    { id: "accepted", text: "Mara lifted the lantern. Its warm handle fitted her hand." },
    { id: "declined", text: "Mara left the lantern on the bench and tightened her coat." },
    { id: "rejoin", text: "Beyond the platform, both paths met beneath the station clock." },
    { id: "departure", text: "The keeper's office was still open. Mara checked the pass in her pocket before deciding where to go." },
  ],
  keeper: [{ id: "p1", text: "The keeper examined pass {pass} and stamped it with a blue star. ‘Keep that number safe,’ he said." }],
  dawn: [{ id: "p1", text: "{consequence}" }],
  river: [{ id: "p1", text: "Along the river, the water carried pale fragments of the morning sky toward the sea." }],
  home: [{ id: "p1", text: "The green lantern marked the road through the mist. When Mara reached home, a light was waiting in the window." }],
  away: [{ id: "p1", text: "At the coast, Mara watched a fishing boat leave the harbour. She opened her map to a place she had never been." }],
};
function record(value: unknown): value is Record<string, unknown> { return typeof value === "object" && value !== null && !Array.isArray(value); }

/** Test content host: bounded external input, existing ResolveRequest/Resolution shapes. */
export function referenceContent(value: unknown): Resolution[] {
  if (!record(value) || !Array.isArray(value.items) || value.items.length > 4096 || !record(value.context)) throw new Error("Invalid content request");
  return value.items.map((item: unknown): Resolution => {
    if (!record(item) || !record(item.content)) throw new Error("Invalid content item");
    const content = item.content;
    const reference = record(content.unit) ? content.unit : content;
    if (reference.provider !== "signal" || typeof reference.key !== "string") return { status: "unavailable" };
    if (!("unit" in content)) {
      const text = labels[reference.key]; return text ? { status: "ok", revision: "signal:1", payload: { text } } : { status: "unavailable" };
    }
    const blocks = bodies[reference.key]; if (!blocks) return { status: "unavailable" };
    const first = content.first == null ? 0 : blocks.findIndex(block => block.id === content.first);
    const last = content.last == null ? blocks.length-1 : blocks.findIndex(block => block.id === content.last);
    if (first < 0 || last < first) return { status: "unavailable" };
    const args = record(item.args) ? item.args : {};
    const pass = record(args.pass) && args.pass.type === "int" && typeof args.pass.value === "string" && /^-?(0|[1-9][0-9]{0,18})$/.test(args.pass.value) ? args.pass.value : "?";
    const signal = record(args.signal) && args.signal.type === "bool" && args.signal.value === true;
    return { status: "ok", revision: "signal:1", payload: { blocks: blocks.slice(first, last+1).map(block => ({ id: block.id,
      text: block.text.replace("{pass}", pass).replace("{consequence}", signal
        ? "At dawn, the lantern Mara accepted on the platform lit the safe road home." : "At dawn, the unmarked road disappeared into mist. Without the lantern, Mara could not find the way home."),
    })) } };
  });
}
