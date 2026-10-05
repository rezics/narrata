import type { MockRezicsData } from "./rezics-content.js";

// The example's original content; only this testing subpath owns body text.
const original = {"camp.fire":{"blocks":[{"id":"p1","text":"{visitor}在火边坐下。守火老人用树枝拨了拨炭，把空出来的地方让给你。"},{"id":"p2","text":"这次停留，老人已招呼过你 {greetings} 次。袋中还剩 {rations} 份干粮；此前你在这里歇过 {rests} 次脚。"}]},"camp.fire.title":{"text":"篝火夜谈"},"camp.letter":{"blocks":[{"id":"p1","text":"老人从衣襟里取出信，抚平封口上的折痕。信封上没有名字，只画着山口石门的轮廓。"},{"id":"p2","text":"“交到守门人的手里就好。”他说完，便把目光移回火上。"}]},"camp.letter.title":{"text":"老者的信"},"camp.rest":{"blocks":[{"id":"p1","text":"你吃了一份干粮，等湿透的衣袖慢慢烤干。老人替你收紧背带，提醒你沿途当心落石。"},{"id":"p2","text":"天边还留着一点亮色。现在可以重新上路了。"}]},"camp.rest.title":{"text":"整理行囊"},"camp.visit:fire.leave.label":{"text":"收拾行囊，回到驿站"},"camp.visit:fire.letter.label":{"text":"询问老人留下的信"},"camp.visit:fire.rest.label":{"text":"吃一份干粮，在火边歇脚"},"camp.visit:fire.rest.reason":{"text":"干粮已经用完"},"camp.visit:letter.continue.label":{"text":"收好信，回到篝火旁"},"camp.visit:rest.continue.label":{"text":"向老人告别"},"camp.visit:title":{"text":"营地互动"},"main.gate":{"blocks":[{"id":"p1","text":"石门从雾里显出轮廓。守门人提着灯走下台阶，先看了看你身后的路，才伸出手。"},{"id":"p2","text":"“驿站那边，有消息吗？”他问。"}]},"main.gate.title":{"text":"抵达山口"},"main.journey:gate.arrive.label":{"text":"告诉他沿途的见闻，结束旅程"},"main.journey:gate.deliver.label":{"text":"将老人的信交给守门人"},"main.journey:gate.deliver.reason":{"text":"需要先从营地老人那里取到信件"},"main.journey:ledger.candle.label":{"text":"带上一截蜡烛"},"main.journey:ledger.flask.label":{"text":"带上一只空水壶"},"main.journey:ledger.rope.label":{"text":"带上一卷麻绳"},"main.journey:ledger.sign.label":{"text":"用炭笔写下自己的名字"},"main.journey:ledger.skip.label":{"text":"合上登记册，不留名字"},"main.journey:station.camp.label":{"text":"到营地歇脚"},"main.journey:station.ledger.label":{"text":"翻看桌上的登记册"},"main.journey:station.road.label":{"text":"沿山路出发"},"main.journey:title":{"text":"主线"},"main.ledger":{"blocks":[{"id":"l1","text":"登记册摊在桌上，纸页被潮气泡得发软。最后一页留着一行空白，旁边压着一截炭笔。"},{"id":"l-sign","text":"你写下自己的名字。炭笔有些钝，最后一笔拖得很长。"},{"id":"l-skip","text":"你把炭笔放回原处。那行空白就留给下一个过路的人。"},{"id":"l2","text":"桌下的木箱里还有前人留下的杂物，也许路上用得着。行囊不大，最多再装两样。"},{"id":"l-candle","text":"你把蜡烛用油纸包好，塞进行囊侧袋。"},{"id":"l-rope","text":"麻绳有些旧，拉了拉，倒还结实。"},{"id":"l-flask","text":"水壶磕瘪了一角，好在不漏。"},{"id":"l3","text":"你合上木箱，把登记册放回原处。屋外的风声小了些。"}]},"main.ledger.title":{"text":"驿站登记册"},"main.station":{"blocks":[{"id":"p1","text":"暮色将山谷染成淡淡的灰紫，旧驿站的木牌在风中轻轻晃动，字迹斑驳，几乎看不清原来的名号。这里曾是往来行旅歇脚换马的所在，现在只剩一间低矮的木屋，屋顶上落满了枯叶。"},{"id":"p2","text":"你推开吱呀作响的木门，屋内空荡，只有一张破旧的桌子。登记册的最后一页写着：往山口去的人，请先到营地找守火老人。他还留着一封信。"},{"id":"p3","text":"夜色渐深，冷风从门缝里钻进来。前方的山路看不清尽头，而不远处似乎有一处营地的火光在摇曳。你需要决定接下来的行动。"}]},"main.station.title":{"text":"旧驿站"},"main:visitor":{"text":"你"},"product:ending":{"blocks":[{"id":"p1","text":"这段旅程已经结束。你可以回到任一历史节点，尝试另一条路线。"}]},"product:ending.title":{"text":"旅程结束"},"product:shared.helped":{"text":"已帮助旅人"},"product:shared.letter":{"text":"已取信件"},"product:shared.packed":{"text":"行囊物件"},"product:shared.rations":{"text":"干粮"},"product:shared.rests":{"text":"歇脚次数"},"product:shared.signed":{"text":"登记册留名"},"product:title":{"text":"山口来信"},"road.crossing:fog.continue.label":{"text":"走向灯火"},"road.crossing:rocks.alone.label":{"text":"独自沿旧路继续"},"road.crossing:rocks.help.label":{"text":"分给旅人一份干粮，请他带路"},"road.crossing:rocks.help.reason":{"text":"至少需要一份干粮"},"road.crossing:rocks.retreat.label":{"text":"先回驿站准备"},"road.crossing:title":{"text":"山路事件"},"road.crossing:trail.continue.label":{"text":"继续前往山口"},"road.fog":{"blocks":[{"id":"p1","text":"你沿旧路慢慢摸索，雾水很快打湿了衣袖。每当碎石从脚边滑下，你便停一停，听它落到底的声音。"},{"id":"p2","text":"山口终于出现在前方，灯火在雾里摇晃。"}]},"road.fog.title":{"text":"迷雾弥漫"},"road.rocks":{"blocks":[{"id":"p1","text":"转过山脚，几块碎石从上方滚落。你扶住岩壁，等响声停下，才发现一个挑着空篮的旅人坐在路边。"},{"id":"p2","text":"他看上去饿了一整天，却认得山口的路。"}]},"road.rocks.title":{"text":"落石滚下"},"road.trail":{"blocks":[{"id":"p1","text":"旅人接过干粮，把空篮重新背好。他没有多问你从哪里来，只领着你拨开一丛低矮的灌木。"},{"id":"p2","text":"这条路绕过了塌方。石缝里透出风，远处已经能看见山口的灯。"}]},"road.trail.title":{"text":"发现小径"}};

export const mountainLetterKeys: Readonly<Record<string, string>> = Object.freeze(Object.fromEntries(
  Object.entries(original).map(([key, entry]) => [key, "blocks" in entry
    ? "https://example.test/occurrences/mountain-letter/" + encodeURIComponent(key)
    : "label:mountain-letter:" + key]),
));

/** Complete original example plus an English realization of its station and ledger scenes. */
export function mountainLetterData(): MockRezicsData {
  const data: MockRezicsData = { occurrences: {}, labels: {} };
  for (const [name, entry] of Object.entries(original)) {
    const key = mountainLetterKeys[name];
    if (!key) throw new Error("Missing example content key");
    if ("blocks" in entry) data.occurrences[key] = { original: "original", realizations: {
      original: { language: "zh-Hans", revision: "original:1:" + name,
        blocks: entry.blocks.map(block => ({ type: "paragraph", attrs: { id: block.id }, text: block.text })) },
    } };
    else data.labels[key] = { original: "original", realizations: {
      original: { language: "zh-Hans", revision: "original:1:" + name, text: entry.text },
    } };
  }
  const translations: Record<string, string[]> = {
    "main.station": [
      "Dusk turns the valley grey. The old station sign swings in the wind; leaves cover the roof of the low wooden house.",
      "You push open the creaking door. The last page of the register reads: If you are heading for the pass, find the old man at the camp first. He still has a letter.",
      "Cold wind slips through the door. Beyond the dark mountain road, a campfire flickers. You must choose where to go.",
    ],
    "main.ledger": [
      "The register lies open, its pages softened by damp. A stick of charcoal rests beside the last blank line.",
      "You write your name. The blunt charcoal leaves a long trail on the final stroke.",
      "You put the charcoal back. The blank line can wait for the next traveller.",
      "A box under the table holds supplies left by earlier travellers. Your bag has room for two more items.",
      "You wrap the candle in oiled paper and tuck it into your bag.",
      "The rope looks old, but holds when you pull it.",
      "The flask has a dent, but does not leak.",
      "You close the box and put the register away. Outside, the wind has eased.",
    ],
  };
  for (const [name, texts] of Object.entries(translations)) {
    const entry = data.occurrences[mountainLetterKeys[name] ?? ""];
    const document = entry?.realizations.original;
    if (!entry || !document) throw new Error("Missing example scene");
    entry.realizations.en = { language: "en", revision: "en:1:" + name,
      blocks: document.blocks.map((block, index) => {
        const text = texts[index];
        if (text === undefined) throw new Error("Incomplete example translation");
        return { type: "paragraph", attrs: { id: block.attrs.id }, text };
      }),
    };
  }
  const labels: Record<string, string> = {
    "product:title": "Letter from the Pass", "main.station.title": "The Old Station", "main.ledger.title": "The Station Register",
    "main.journey:station.camp.label": "Rest at the camp", "main.journey:station.ledger.label": "Read the register on the table",
    "main.journey:station.road.label": "Take the mountain road", "main.journey:ledger.sign.label": "Write your name with the charcoal",
    "main.journey:ledger.skip.label": "Close the register without signing", "main.journey:ledger.candle.label": "Take a candle",
    "main.journey:ledger.rope.label": "Take a coil of rope", "main.journey:ledger.flask.label": "Take an empty flask",
  };
  for (const [name, text] of Object.entries(labels)) {
    const entry = data.labels[mountainLetterKeys[name] ?? ""];
    if (!entry) throw new Error("Missing example label");
    entry.realizations.en = { language: "en", revision: "en:1:" + name, text };
  }
  return data;
}
