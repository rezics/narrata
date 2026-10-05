import { createReader, type ContentResolver, type ReaderBook } from "@rezics/narrata";
import { readerMessages, type ContentSlotProps, type ReaderClasses, type ReaderMessages, type ReaderSlots } from "@rezics/narrata/react";
import { reader } from "./client";
import type { ViewReply } from "./protocol";

export const messages: ReaderMessages = {
  ...readerMessages,
  loading: "正在打开故事……", unavailable: "内容暂不可用", incompatible: "此内容无法显示",
  error: "暂时无法更新故事，请重试。", retry: "重试", reload: "重新载入存档",
  memory: "尚未保存到本机", saving: "正在保存", saved: "保存到本机", failed: "保存失败，请重新载入存档后继续。",
  superseded: "另一页面已更新这份旅程。请重新载入存档后继续，当前操作未保存。", reloading: "正在重新载入存档",
  choices: "当前可执行行动", path: "旅程时间线", state: "变量", previous: "上一步", next: "下一步", finished: "故事结束",
  selection: (min, max) => min === 0 ? `最多选择 ${max} 项，也可以都不选` : min === max ? `选择 ${min} 项` : `选择 ${min}–${max} 项`,
  submit: count => count === 0 ? "都不选，继续" : `确定（${count} 项）`,
};
export const classes: ReaderClasses = {
  reader: "reading", heading: "reading-heading", productTitle: "chapter-label", prose: "prose", lead: "lead-in",
  reply: "reply", subtitle: "passage-subtitle", choices: "choices", multi: "multi", choiceItem: "choice-item", choice: "choice",
  choiceCheck: "choice-check", reason: "choice-reason", submit: "primary-button choice-submit", footer: "reading-footer",
  previous: "outline-button", next: "primary-button", save: "save-status", path: "history", error: "message error",
};

/** Local text is just this host's rendering policy; another host can mount DocumentBody here. */
function localContent(props: ContentSlotProps) {
  const { resolution } = props;
  if (resolution.status !== "ok") return <span className="missing" data-content-status={resolution.status}>{messages[resolution.status]}</span>;
  if ("text" in resolution.payload) return resolution.payload.text;
  return <>{resolution.payload.blocks.map(block => <p key={block.id} data-anchor={block.id}>{block.text}</p>)}</>;
}
export const slots: ReaderSlots = { body: localContent, option: localContent, reference: localContent };

export function referenceReader(initial: ViewReply, onView: (reply: ViewReply) => void) {
  let latest = initial;
  const accept = (reply: ViewReply) => { latest = reply; onView(reply); };
  const command = async (input: Parameters<typeof reader.call>[0]) => {
    const reply = await reader.call(input);
    if (reply.kind !== "view") throw new Error("Expected a view from the runtime");
    accept(reply);
    return reply.book;
  };
  const book: ReaderBook = {
    inspect: async () => latest.book,
    choose: async (expected, choice_point, options) => (await command({ kind: "choose", expected, choice_point, options: [...options] })).view.cursor,
    checkout: async commit => { await command({ kind: "checkout", commit }); },
    reload: async () => { await command({ kind: "reload" }); },
    nextContentUnits: async () => {
      const reply = await reader.call({ kind: "lookahead" });
      if (reply.kind !== "units") throw new Error("Expected content units from the runtime");
      return reply.units;
    },
  };
  const resolver: ContentResolver = { resolve: async request => {
    const reply = await reader.call({ kind: "resolve", request });
    if (reply.kind !== "resolved") throw new Error("Expected content from the runtime");
    return reply.results;
  } };
  const controller = createReader(book, resolver, { context: { languages: [...navigator.languages] },
    persistence: initial.saved_at ? "durable" : "memory", savedAt: initial.saved_at });
  return { controller, accept };
}
