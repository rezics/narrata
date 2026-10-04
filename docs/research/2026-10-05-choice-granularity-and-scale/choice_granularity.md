# Choice Granularity and Scale: Is "passage = node, choices only at passage end" best practice, and what does it cost?

Scope note: These notes test a proposed Narrata/REZICS split. Every narrative passage is a separate REZICS "chapter" content unit. Choices appear only at the end of a chapter, and each option loads another chapter. Chapter text documents contain no nested choice trees: the graph lives in Narrata, the text lives in REZICS. The target scale is ~100,000 choice points per work.

Sources covered: tool docs (ink, Yarn Spinner, Twine story formats, ChoiceScript, articy:draft, Ren'Py), inkle postmortems and articles, Sam Kabo Ashwell and Emily Short, Choice of Games, Failbetter, Pixelberry and Episode creator docs, and Chinese creator docs (易次元, 橙光, 闪艺, 口袋方舟). Research date: 2026-10-05.

Marking conventions:
- **[P]** primary source (official doc, developer post or talk).
- **[secondary]** press, encyclopedia or search-engine excerpt.
- **[community]** forum or fan wiki.
- **[own analysis]** numbers I computed from the cited data.
- *Inference* marks my reasoning, as opposed to what a source says.

Quotes are short; most findings are paraphrased.

Related notes: `../叙事引擎核心与可插拔存储/industry_tools_formats.md` covers logic/text separation, line IDs and storage. `long_and_live_narratives.md` covers Fallen London and AAA scale.

---

## Q1. What unit do choices connect in major IF tools and formats? Which allow choices mid-passage or nested, and why?

### Takeaway
There are two families:
- **End-of-unit choices:** gamebook sections, early inklewriter and early ink, Twine passages (by default), StoryNexus/Fallen London storylets, 闪艺, and 口袋方舟. Choices sit at the end of a text unit and point to other units.
- **Units with an in-unit choice construct that rejoins automatically:** ink weave, ChoiceScript `*choice`/`*fake_choice`, Yarn `->` options, Ren'Py `menu`, Episode choice blocks, 易次元 "分支" options, and 橙光 文字选项. Even Twine's story formats add in-passage reveal/append.

Every widely used modern tool that started node-only later added an in-unit choice mechanism. The stated reasons are authoring speed, robustness (no loose ends), easy redrafting, and not discouraging small choices.

### Cited Findings

#### Comparison matrix

| System | Unit that choices connect | Choices mid-unit / nested? | Rejoin mechanism | Source |
|---|---|---|---|---|
| Classic gamebooks (Fighting Fantasy, Lone Wolf, CYOA) | Numbered section or page | No. Choices come at the end of a section. | Many sections point to the same section number | Adventure gamebook sections are usually no longer than a paragraph or two, with choices at the end of each section ([Wikipedia: Gamebook](https://en.wikipedia.org/wiki/Gamebook) [secondary]). Warlock of Firetop Mountain has 400 sections ([Wikipedia](https://en.wikipedia.org/wiki/The_Warlock_of_Firetop_Mountain) [secondary]). Most Lone Wolf books have 350 ([Wikipedia: Lone Wolf](https://en.wikipedia.org/wiki/Lone_Wolf_(gamebooks)) [secondary]). The Cave of Time has 40 endings over 114 pages ([Ashwell 2011](https://heterogenoustasks.wordpress.com/2011/08/05/cyoa-structures-the-cave-of-time/) [P]). |
| inklewriter (inkle web tool, 2012) | Paragraph ("scrap of paper") | No. Options are added under a paragraph, and each option arrow leads to a paragraph. | A "join" button links a branch back to an existing paragraph | [inklewriter Getting Started](https://www.inklestudios.com/inklewriter/getting-started/) [P] |
| Early ink (Sorcery! 1–2) | Knot/stitch sections linked by "goto" arrows, with asterisk choices | Initially goto-based | Explicit diverts | Joseph Humfrey, [Game Developer, 2016-03-30](https://www.gamedeveloper.com/design/open-sourcing-80-days-narrative-scripting-language-ink) [P] |
| ink (current) | Knot (scene) → stitch (event in scene) → **weave** | **Yes.** Choices (`*`) and gathers (`-`) nest by indentation. Tunnels (`-> t ->`) run a sub-story and return. | Gathers automatically rejoin the paths. Flow always falls downward. | [WritingWithInk.md](https://github.com/inkle/ink/blob/master/Documentation/WritingWithInk.md) [P] |
| Twine 2 (Harlowe / SugarCube / Chapbook) | Passage plus links | Formats add in-passage interaction. Harlowe has `(link-reveal:)`, `(click-append:)` and `(display:)`. SugarCube has `<<linkappend>>`/`<<linkreplace>>` (since 2.8.0, 2016). Chapbook has `{reveal link: …, text/passage: …}`. | Links back to a shared passage, or reveal-in-place | [Harlowe manual](https://twine2.neocities.org/) [P]; [SugarCube docs](https://www.motoslave.net/sugarcube/2/docs/) [P]; [Twine Cookbook: Chapbook passages-in-passages](https://twinery.org/cookbook/passagesinpassages/chapbook/chapbook_passagesinpassages.html) [P] |
| ChoiceScript (Choice of Games) | Scene file (chapter-like, ordered by `*scene_list`). Inside it, labels. | **Yes.** `*choice` lists `#options`, and each result is an indented block that may contain a nested `*choice`. `*fake_choice` needs no `*goto`/`*finish` and simply continues. | Fall through after `*fake_choice`; `*goto label` | [CoG ChoiceScript intro](https://www.choiceofgames.com/make-your-own-games/choicescript-intro/) [P]; [CoG commands](https://www.choiceofgames.com/make-your-own-games/important-choicescript-commands-and-techniques/) [P] |
| Yarn Spinner | Node | **Yes.** `->` options with indented bodies, nestable. Lines after the option group run whatever was chosen. The docs suggest `<<jump>>` to another node once nesting gets hard to read. | Implicit fall-through after the option group | [Yarn docs: Options](https://docs.yarnspinner.dev/write-yarn-scripts/scripting-fundamentals/options) [P]; [Yarn 2.5 docs: Nodes, Lines, Options](https://docs.yarnspinner.dev/2.5/getting-started/writing-in-yarn/lines-nodes-and-options) [P] |
| Yarn Spinner 3 | Node, **node group** (storylets with a `when:` header) and **line group** (`=>`, one salient line chosen) | Line groups can nest content | Saliency selection, not player choice | [Yarn 3.0 blog, 2024-01-24](https://yarnspinner.dev/blog/yarn-spinner-30-what-to-expect/) [P]; [Storylets primer](https://docs.yarnspinner.dev/write-yarn-scripts/advanced-scripting/storylets-and-saliency-a-primer) [P] |
| articy:draft | Flow fragment / Dialogue as **containers** (you can "submerge" into them). A Dialogue Fragment is a single line. | A choice is several outgoing connections from fragments. Nesting goes from chapter to scene to dialogue to line. Dialogue fragments cannot contain inner content. | Hubs and connections | [articy nesting tutorial](https://www.articy.com/en/tutorial-nesting/) [P]; [Dialogue fragments](https://www.articy.com/help/legacy/Flow_Objects_DialogFragment.html) [P] |
| Ren'Py | Label | **Yes.** `menu:` choices with statement blocks. When a block ends, execution continues after the menu. `jump`/`call` go to labels. | Fall-through after `menu` | [Ren'Py In-Game Menus](https://www.renpy.org/doc/html/menus.html) [P] |
| Fallen London / StoryNexus | Storylet (root event) → branches (choices) → outcome events (success/failure) | One choice layer per storylet, then back to the hub/deck | The hub (location deck) acts as the bottleneck | Staff statistics below (Q3) [P] |
| Episode (Pocket Gems) | Episode (chapter) script | **Yes.** `choice` with option text and a braced dialogue block. `label`/`goto` handle complex branching. | Branches merge after the choice block | [Episode WP Guide 16](https://pocketgems-support.helpshift.com/hc/en/10-episode-writer-s-portal/faq/368-writer-s-portal-guide-16-basic-choices-and-branching/) [P]; [Labels & Branches](https://pocketgems-support.helpshift.com/hc/en/10-episode-writer-s-portal/faq/402-labels-branches/) [P] |
| 易次元 (NetEase) | 剧情段 (story segment) made of 画布 (canvases), managed in a 剧情目录 tree | **Yes.** Options added via **分支** live *inside the segment*, and the option branch can hold new canvases. Options added via **控件** are canvas-level click events (jump/插播/variable ops). | 剧情插播 = call-and-return. 剧情跳转 = jump. Segments play top-down by default. | 易次元创作学院: [剧情的跳转和插播](https://wap.avg.163.com/creator/school/course/69) (2023-09-22) [P]; 添加选项, 分支, 剧情目录 tutorials (same site, course ids 152, 72, 46, read via the public course API) [P] |
| 橙光 (66rpg) classic tool | 剧情 inside a 剧情树 / chapter list | **Yes.** A 文字选项 is inserted at a frame inside a 剧情, and each option's follow-up content is added after that option. 剧情跳转 jumps between 剧情. | Inline continuation. Advanced mode's "呼叫子剧情" returns to the main 剧情 [secondary Q&A]. | [66rpg: 如何在初级模式下添加文字选项](https://www.66rpg.com/course/details/t_92/332.shtml) [P]; [喜马拉雅 Q&A](https://m.ximalaya.com/ask/q7363132) [community] |
| 闪艺 (Shanyi) | 剧情 = **one screen** (one 画面 equals one 剧情), grouped into 章节 | Options branch between 剧情 | Jumps | [闪艺教程](https://www.3000.com/course.html) [P] |
| 口袋方舟 (Ark.online; migration guide for 橙光 creators) | 章 (chapter) → 节 (section) | **Mostly end-of-unit.** Sections auto-continue or use "抛出选项". Chapter-to-chapter transitions *must* use a direct jump or a thrown option, configured as an end-of-section jump. | Jumps by chapter index; "生成流程图" to check links | [learning.ark.online 橙光创作者快速入门指南](https://learning.ark.online/Getting-Started/developer66RPG-guide.html) [P] |

#### Why inline choices exist (stated rationales)
- **ink weave.** The manual's case for weave is that linking "options" to "pages" forces you to *uniquely name every destination*. That slows writing and can *discourage minor branching*. Weave flow is guaranteed to start at the top and fall to the bottom, so a basic weave cannot have flow errors. Weaves also make it easy to split a sentence and insert extra choices "for variety or pacing" without re-engineering any flow. Nested weaves eventually get hard to read, and the manual's style advice is then to divert to a new stitch. — [WritingWithInk.md](https://github.com/inkle/ink/blob/master/Documentation/WritingWithInk.md) [P]
- **inkle history.** Early ink linked sections with goto arrows. That "worked well enough" for Sorcery!, but adding weave was *the biggest change in the early days* and was pivotal to the writing style of 80 Days. In a weave it is impossible to have "loose ends", which are common when a story is linked by goto arrows. — Joseph Humfrey, [Game Developer (2016-03-30)](https://www.gamedeveloper.com/design/open-sourcing-80-days-narrative-scripting-language-ink) [P]
- **Jon Ingold on flowcharts** ("Introduction to Ink", Wireframe, republished on Medium on 2020-02-20). Laid out as a chart, the first scene of 80 Days takes a whole screen of diagram for one minute of gameplay. Adding a choice mid-branch means taking the graph apart to insert nodes; the ink version is compact and quick and safe to redraft. Ingold adds that 80 Days' script is over 600,000 words and 12,000 "nodes" of content. — [Medium / Game Writing Guide](https://medium.com/game-writing-guide/introduction-to-ink-3e6c224865f8) [P] (text read via the publication's RSS feed, since the article page returned 403)
- **ChoiceScript.** `*fake_choice` exists so an option can carry text without a `*goto`/`*finish`; flow just continues ([CoG commands](https://www.choiceofgames.com/make-your-own-games/important-choicescript-commands-and-techniques/) [P]). Choice of Games' taxonomy separates Fake, Flavor, Establishing, Objective and Forking choices. Only forking choices create mutually exclusive branches. Flavor choices record preferences that shouldn't change outcomes. — [CoG: A Taxonomy of Choices (2017)](https://www.choiceofgames.com/2017/12/a-taxonomy-of-choices-establishing-character/) [P]. Dan Fabulich (2010): a game in which every option branches into a different story is impossible to finish. Choices often change stats instead of branching immediately. — [CoG: 5 Rules (2010)](https://www.choiceofgames.com/2010/03/5-rules-for-writing-interesting-choices-in-multiple-choice-games/) [P]
- **Episode.** After a choice block, the two branches merge again, so every reader sees the next line ([WP Guide 16](https://pocketgems-support.helpshift.com/hc/en/10-episode-writer-s-portal/faq/368-writer-s-portal-guide-16-basic-choices-and-branching/) [P]). The Complex Branching guide advises merging branches early, not leaving the reader in a side branch for long, and using flags to remember the choice. [secondary: search excerpt of the official page [Complex Branching](https://episodesupport.zendesk.com/hc/en-us/articles/115004056594-Complex-Branching); the page returned 403 to direct fetch]. After a merge, flags can be checked at any time ([Checking Flags](https://pocketgems-support.helpshift.com/hc/en/10-episode-writer-s-portal/faq/350-checking-flags---remembering-previous-choices/) [P]).
- **易次元.** The official tutorial gives two ways to add options and *recommends 分支*. 分支 options act within the 剧情, and their branches can hold new canvases and story directly; the tutorial says this is the method used for option story content. 控件 options are single-canvas click events (jump, 插播, variable op, UI, sound) "for simple click events". — 易次元创作学院 "添加选项" (course 152, updated 2023-09-22) [P]. Limits: at most **200 canvases per 剧情** (course 46) and a **260,000 code limit per 剧情** in advanced mode (course 76). [P]
- **橙光 official journal ("剧情向作品如何设置选项").** Options should start early. Options whose every path leads to the same result shouldn't exist. Short-line branches can return to the main line after a branch passage. — [66rpg 作品提升刊](https://www.66rpg.com/t_107/nuBRkeVd34.shtml) [P; paraphrased by fetch tool]

### Inferences
- The proposed REZICS rule matches the **gamebook / inklewriter / StoryNexus / 口袋方舟** family. That model is legitimate and long-lived, but it is the model inkle moved *away* from for dialogue and local reactivity.
- No mainstream modern authoring format forbids in-unit choices. The closest are the 口袋方舟 chapter rule and 闪艺's screen-per-node, and both treat a "unit" as a small screen or section, not a novel chapter.

### Gaps
- No primary 快点 or Chapters (Crazy Maple) creator documentation on in-chapter branching was reachable.
- The Harlowe and Chapbook doc wording above is paraphrased via a fetch tool, not verbatim.

---

## Q2. Why did inkle create the weave, and how often are choices cosmetic or quickly rejoining?

### Takeaway
inkle's statements are explicit. A node per choice forces naming, slows writing, discourages minor branching, produces loose ends, and makes redrafting expensive. Their games use *many small choices*: in Sorcery!, roughly one choice per ~100 words, some of them pure flavor. Field data from a chapter-based commercial platform (Choices) shows that **about 22 choice points per chapter is normal**, that **about 55% of choice points have no recorded effect on any option**, and that **only about 3% route to distinct paths**. Almost every choice rejoins inside the same chapter.

### Cited Findings
- **Sorcery! density.** Jon Ingold's postmortem (2016-09-22) says Part 1 had *small choices every 100 words or so*, many more than the original gamebook, and some were mere flavor. — [Game Developer](https://www.gamedeveloper.com/business/postmortem-i-steve-jackson-s-sorcery-i-series-by-inkle) [P]. The same postmortem says the 4-game series had just over 60k significant lines of ink and nearly 1.5M words ([Q&A 2016](https://www.gamedeveloper.com/design/q-a-jon-ingold-on-i-sorcery-i-and-crafting-interactive-fiction) [P]).
- **80 Days pacing.** Ingold (2014) describes a conversational pattern: you say something, the game says something back. Sequences of small approach-choices build tension. — [Game Developer, 2014-08-05](https://www.gamedeveloper.com/business/-i-80-days-i-building-the-perfect-text-adventure-for-mobile) [P]
- **Ashwell, "Standard Patterns in Choice-Based Games" (2015-01-26)** ([P](https://heterogenoustasks.wordpress.com/2015/01/26/standard-patterns-in-choice-based-games/)):
  - *Time Cave*: heavy branching with little or no re-merging, which explodes.
  - *Gauntlet*: a linear thread pruned by death, backtracking or **quick rejoining**. Ashwell calls it perhaps the easiest structure to author.
  - *Branch and Bottleneck*: branches regularly rejoin at common events, with heavy state-tracking.
  - *Sorting Hat*, *Quest*, *Open Map*, *Floating Modules*, *Loop and Grow*, *Spoke and Hub*.
- **Ashwell, "A Bestiary of Player Agency" (2014).** Aesthetic choice promises nothing more than ownership. Reflective choices change nothing in the mechanics. — [P](https://heterogenoustasks.wordpress.com/2014/09/22/a-bestiary-of-player-agency/)
- **Emily Short, "Small-Scale Structures in CYOA" (2016).** Local patterns that live *inside* a scene: confirmation-required choice, track-switching choice, scored choice, re-enterable conversation node, chapter-one sorting hat, endgame time cave. — [P](https://emshort.blog/2016/11/05/small-scale-structures-in-cyoa/)
- **Bruno Dias, "The Branch and the Merge" (sub-Q, 2018).** Merging is a necessary counterpart of branching past a certain length. Hide merge seams with scene breaks and small text adaptations. — [P](https://sub-q.com/making-interactive-fiction-the-branch-and-the-merge/)
- **Choices (Pixelberry): measured flavor and rejoin rates** [own analysis of [community] wiki data]. I parsed the community-maintained "Book 1 Choices" walkthrough pages of the Choices fandom wiki through the MediaWiki API. Setup chapters are excluded, and a "choice point" is a choice with ≥2 options. The wiki labels outcomes with tags such as "(No effect)", "(Path A)" and "(Go to Choice N)".

| Book 1 | Chapters | Choice points | Per chapter | All options "No effect" | Path/goto routing | Avg options |
|---|---|---|---|---|---|---|
| The Royal Romance | 19 | 506 | 26.6 | 66% | 3.4% | 2.52 |
| Desire & Decorum | 16 | 501 | 31.3 | 43% | 1.6% | 2.43 |
| Endless Summer | 16 | 248 | 15.5 | 13% | 6.5% | 2.45 |
| Open Heart | 17 | 482 | 28.4 | 87% | 2.5% | 2.21 |
| The Crown & The Flame | 18 | 312 | 17.3 | 38% | 4.8% | 2.40 |
| The Freshman | 17 | 238 | 14.0 | 55% | 3.4% | 2.37 |
| **Total** | **103** | **2,287** | **22.2** | **54.7%** | **3.3%** | **2.40** |

  Notes on the table:
  - "No effect" means no stat or relationship change. Most such options still carry a distinct response line before the story rejoins, which is the classic weave pattern.
  - The remaining ~42% mostly adjust relationship or stat values and then rejoin. Endless Summer is relationship-heavy, which is why its "No effect" share is low.
  - Choices after path splits are numbered "Choice 12 or 13" on the wiki, which shows that paths rejoin within the chapter.
  - Caveats: the annotations are fan-made, the parser is heuristic, and diamond-scene sub-choices are included.
  - Sources: [wiki API for TRR Book 1](https://choices-stories-you-play.fandom.com/wiki/The_Royal_Romance,_Book_1_Choices) and the same API for the other five pages [community].
- **Pixelberry scale.** Choices passed **1,000 released chapters** in Sept 2019 ([Pixelberry blog](https://www.pixelberrystudios.com/blog/2019/9/30/1000-chapters-and-counting) [P]). Typical chapters run ~5–15 minutes, and books run ~15–19 chapters ([SV forums essay](https://forums.sufficientvelocity.com/threads/choices-stories-you-play-by-pixelberry-studios-an-introduction-a-review-a-critique-and-other-ramblings.154287/) [community]). The six Book 1s above have 16–19 chapters.
- **Choice frequency guidance at CoG** [community]:
  - Forum advice is ~200 words between choices, avoiding >400, with `*fake_choice` used to break up text ([CoG forum](https://forum.choiceofgames.com/t/how-long-is-too-long-to-go-without-a-choice/58610)).
  - Another forum answer says there is no rule, citing a game with about one choice per 2,500 words ([CoG forum, 2025-09](https://forum.choiceofgames.com/t/design-guidelines-for-hosted-games/173294)).

### Inferences
- The *dominant* choice in chapter-based commercial IF is a **local choice that rejoins within a few lines**: a flavor reply, a stat or relationship nudge, or a premium mini-scene. Structural forks are a small minority, roughly 2–7% by the Choices data.
- inkle's 80 Days ratio is ~600k words per 12,000 "nodes", about 50 words per addressable unit. That suggests inkle's own "node" count sits near choice granularity, but writers never *authored* nodes one by one. Weave generated them.

### Gaps
- No public `*choice` vs `*fake_choice` counts exist for CoG titles. That needs source access, which CoG titles don't publish.
- No primary Pixelberry data on in-chapter structure. The wiki-derived figures should be re-checked if this decision hinges on them.

---

## Q3. How many passages or nodes do large works have? Do node-per-passage tools struggle at thousands of nodes?

### Takeaway
The largest IF works reach 10^4 nodes, not 10^5:
- Fallen London: 45,718 events / 29,390 branches / 8,351 root storylets after 15 years.
- 80 Days: 12,000 nodes.
- Zombie Exodus: Safe Haven: 2.7M words.

A target of **100,000 choice points in one work is about 12× Fallen London's root storylets and about 3× its branch count**. Editors that show every passage as a box degrade at **~600–1,600 passages**.

### Cited Findings
- **Fallen London (Failbetter staff, 2024-07-23)**: 8,351 root events; 45,718 total events; 29,390 total branches. Words: 2,722,469 in events, 711,515 in branches, and 1,037,338 in qualities, for ~4.47M total. — [Failbetter forum (staff post)](https://community.failbettergames.com/t/fallen-london-has-almost-4-5-milion-words/22727) [P]. **[own analysis]**: 3.52 branches per root storylet. There are 1.27 outcome events per branch, excluding roots (success/failure variants). **Total events ÷ root storylets ≈ 5.5.**
- **80 Days**: 600,000+ words and 12,000 "nodes" ([Ingold 2020](https://medium.com/game-writing-guide/introduction-to-ink-3e6c224865f8) [P]). It later reached ~750k words ([inkle blog 2015](https://www.inklestudios.com/2015/09/17/new_adventures.html) [P]). inkle's engine had to **stream story content on demand** instead of loading the whole script up-front, which was work done for Sorcery! 2's 350k words. — [80 Days postmortem, 2015-10-14](https://www.gamedeveloper.com/business/postmortem-inkle-s-i-80-days-i-) [P]
- **Sorcery!**: nearly 1.5M words across four parts; 695 variables carried from Part 3 into Part 4 ([Game Developer Q&A 2016](https://www.gamedeveloper.com/design/q-a-jon-ingold-on-i-sorcery-i-and-crafting-interactive-fiction) [P]).
- **Choice of Games**:
  - CoG targets ≥100k total words and ≥20k words per average playthrough. The sweet-spot ratio of playthrough to total is 0.2–0.4. Below 0.2 means the author is writing content most readers never see. — [CoG: Length and Coding Efficiency (2017)](https://www.choiceofgames.com/2017/07/length-and-coding-efficiency/) [P]
  - *Zombie Exodus: Safe Haven* exceeds **2,700,000 words** in total and ~210,000 per full playthrough (ratio ~0.08). Part Four was released 2026-02-05. — [CoG blog](https://www.choiceofgames.com/2026/02/new-hosted-game-zombie-exodus-safe-haven-part-four/) [P]
  - Choice of Rebels is ~637k words and Choice of Robots ~300k ([CoG forum list](https://forum.choiceofgames.com/t/list-of-all-stories-by-word-length/32361) [community]).
- **Twine corpora (passages per work and link density)**:
  - *Undergraduate Games Corpus* (Anderson & Smith, AAAI): 207 Twine games, 11,822 passages (~57 per game), ~1.5 references per passage. The authors note the most common out-degree is **one** (linear story segments), with a long tail at choice scenes. — [AAAI PDF](https://ojs.aaai.org/index.php/AAAI/article/download/16071/15878) [P]
  - *Spindle* (Calderwood, Wardrip-Fruin, Mateas, ICCC 2022): of 512 itch.io Twine stories, 82 were decompilable. They produced 10,784 passages with 11,098 links, after excluding macro passages. That is ~131 passages per story and ~1.03 links per passage. — [ICCC 2022 PDF](https://computationalcreativity.net/iccc22/papers/ICCC-2022_paper_65.pdf) [P]
- **Twine editor limits**:
  - twinejs #766 (2021-01-13, closed): lag at "more than 600–800 passages" or many arrows. — [GitHub](https://github.com/klembot/twinejs/issues/766) [P / community report]
  - intfiction (2020-11): TheMadExile says the lag is mostly the story map drawing connection arrows. HiEv suggests ~700 passages before the editor gets too laggy, and says Tweego is unaffected by passage count. — [intfiction](https://intfiction.org/t/performance-with-so-many-passages/48097) [community]
  - Agnieszka Trzaska (2022-12-26): the editor slows at about 1,600 passages and 2,500 links. — [intfiction](https://intfiction.org/t/lots-of-small-games-or-one-big-game/59370) [community]
  - Release notes: 2.1.0 (2017) made loading large stories considerably faster, and 2.4.0 (2022-07-05) made "Editing larger stories is faster" ([Twine 2.4.0 notes](https://twinery.org/cookbook/releasenotes/twine2/2.4.0.html) [P]). No 2.9–2.11 release notes mention large-story performance ([2.9](http://twinery.org/reference/en/release-notes/2-9.html), [2.11](https://twinery.org/reference/en/release-notes/2-11.html) [P]).
- **articy:draft at scale**: Disco Elysium's dialogue volume made articy "janky" and at one point froze it (see the sibling note `industry_tools_formats.md` Q5, citing [PC Gamer/Yahoo](https://tech.yahoo.com/gaming/articles/disco-elysium-had-much-text-032304926.html)). articy recommends nesting once a project grows too big for one flow layer ([articy nesting](https://www.articy.com/en/tutorial-nesting/) [P]).
- **Chinese platforms' scale**:
  - 橙光 had ~55M registered users, 3M+ creators and ~10M works by end of 2018 ([游戏陀螺](https://www.youxituoluo.com/520390.html) [secondary]).
  - Individual works exceed 1M characters, e.g. 《从此再无天人书-碎梦篇》 at 1,068,617 字 ([66rpg work page](https://www.66rpg.com/game/1041625) [P]).
  - 易次元 caps one 剧情 at 200 canvases and 260k code (above), which forces large works into many segments.
- **Episode**: an episode must be 400–18,000 script lines ([Episode forums](https://forums.episodeinteractive.com/t/is-there-a-limit-on-how-long-each-episode-should-be/681) [community, search excerpt]).

### Inferences
- **No public example of 10^5 independently authored passages in one work was found.** The proposed scale is beyond Fallen London's 15 years of content in choice count. Per-unit overhead in storage, metadata, IDs, editing UI and reader fetches therefore matters more than in any precedent.
- Flat graph editors fail between ~10^2.8 and 10^3.2 nodes. At 10^5–10^6 units, authoring UIs must be hierarchical (articy nesting, 易次元 剧情目录 folders, 闪艺 章节) or text-first (ink, Twee/Tweego).

### Gaps
- No primary passage counts for the largest Twine works (Degrees of Lewdity, Spy Intrigue).
- No public node or choice counts for Choices, Episode or 橙光 top titles beyond words and chapters.

---

## Q4. How do commercial chapter-based platforms (Choices, Episode, Chapters, Tap, 橙光, 易次元, 闪艺, 快点) organize chapters vs choices?

### Takeaway
On Western mobile platforms (Choices, Episode, Chapters), a **chapter or episode is a 5–15 minute unit with ~14–31 choices inside**. Almost all of them branch and rejoin within the chapter, and chapter boundaries act as bottlenecks. Chinese AVG platforms split two ways:
- **In-segment option blocks**: 易次元 "分支" (recommended), 橙光 文字选项.
- **Fine-grained nodes grouped into chapters**: 闪艺 (one screen per node), 口袋方舟 (章/节 with end-of-section options).

None of them treats a reader-facing "chapter" as a single passage with one terminal choice.

### Cited Findings
- **Choices (Pixelberry)**: about 22 choice points per chapter across 103 chapters, about 55% "No effect", and about 3% path routing (Q2 table) [own analysis of community data]. There were 1,000 chapters by 2019-09 ([P](https://www.pixelberrystudios.com/blog/2019/9/30/1000-chapters-and-counting)).
- **Episode**: episodes are script files with inline braced choice blocks that merge afterward. `label`/`goto` handle complex routing, and flags keep memory after a merge ([P](https://pocketgems-support.helpshift.com/hc/en/10-episode-writer-s-portal/faq/368-writer-s-portal-guide-16-basic-choices-and-branching/), [P](https://pocketgems-support.helpshift.com/hc/en/10-episode-writer-s-portal/faq/350-checking-flags---remembering-previous-choices/)).
- **Chapters (Crazy Maple)**: a browser-based Writing Room for branching stories ([P](https://ugc.crazymaplestudios.com/); [PR Newswire](https://www.prnewswire.com/news-releases/crazy-maple-studio-announces-narrative-game-publishing-platform-301354917.html) [secondary]). No public doc on in-chapter structure was found.
- **Tap by Wattpad (2017)**: chat-style stories advanced by tapping. A choice mechanic was added later ([Wattpad company blog 2017-03-22](https://company.wattpad.com/archives/2017-3-22-chat-style-stories-just-got-better-with-the-latest-tap-by-wattpad-update) [P]; [Tim Chisholm portfolio](http://www.timpchisholm.com/tap-by-wattpad-2018) [secondary]).
- **易次元**: works are trees of 剧情 segments (with folders), played top-down unless a jump or 插播 is set. Options are either in-segment 分支 blocks (recommended for option content) or canvas click events. 插播 returns to the calling canvas. Word counts are shown per segment and per work. [P, 创作学院 courses 46/69/152]
- **橙光**: options are inserted inside a 剧情 with per-option follow-up content, and 剧情跳转 links 剧情 ([P](https://www.66rpg.com/course/details/t_92/332.shtml)). Its "选项日志" back-end shows per-option pick rates ([66rpg 作品提升刊](https://www.66rpg.com/t_107/nuBRkeVd34.shtml) [P]).
- **闪艺**: one screen equals one 剧情, grouped in 章节 ([P](https://www.3000.com/course.html)).
- **口袋方舟**: 章 → 节. Sections auto-continue or end with thrown options. Chapter transitions must be a direct jump or a thrown option ([P](https://learning.ark.online/Getting-Started/developer66RPG-guide.html)).
- **快点阅读**: chat-style 对话小说 with multiple endings ([App Store](https://apps.apple.com/cn/app/%E5%BF%AB%E7%82%B9%E9%98%85%E8%AF%BB/id1228112060) [secondary marketing]). No creator doc on branching structure was found.

### Inferences
- Where a platform uses fine nodes (闪艺, 口袋方舟), the node is a **screen or section**, and the reader-facing "chapter" is a **container** of many nodes. That is a two-level model, not "node = chapter".
- On REZICS, the closest analogue is a reader-facing chapter that contains many engine nodes. If REZICS "chapter" means both the content unit *and* the reader-facing chapter, the Choices data implies **~75 REZICS chapters per Choices-style chapter** (see Q6).

### Gaps
- 快点, Chapters and Hooked creator tooling details.
- Per-chapter choice counts for 橙光/易次元 top works.

---

## Q5. Reader UX: must "load a new chapter" be a page break?

### Takeaway
No. Many systems render many small nodes as **one continuous, appended flow**, and node boundaries are invisible to the reader:
- ink's default web template; Twine 1's Jonah; Undum
- Harlowe, SugarCube and Chapbook reveal/append
- chat fiction (Tap, 快点)

Page-per-choice presentation (ChoiceScript, Fallen London, gamebooks) is a design choice, not a consequence of the data model.

### Cited Findings
- **ink web template**: text accumulates in a continuously scrolling page. A `# CLEAR` tag is needed to wipe it, and `# RESTART` resets. Choices are styled paragraphs with links. — [inkle web tutorial](https://www.inklestudios.com/ink/web-tutorial/) [P]. A community template lets authors toggle between endless scroll and page flipping with a `#toggleFlow` tag ([ink-soaked](https://github.com/wickedlyethan/ink-soaked) [community; search excerpt, repo page 404 at fetch time]).
- **Twine 1 Jonah**: as the player clicks links, the text expands. Earlier passages can be reviewed by scrolling up ("stretch-text"). — [Twine Cookbook: Jonah](https://twinery.org/cookbook/twine1/storyformats/jonah/index.html) [P]
- **Harlowe**: `(link-reveal:)` and `(click-append:)` append text in place, and `(display:)` embeds another passage without changing the current passage ([Harlowe manual](https://twine2.neocities.org/) [P]). SugarCube's `<<linkappend>>` and `<<linkreplace>>` do the same ([SugarCube docs](https://www.motoslave.net/sugarcube/2/docs/) [P]). Chapbook's reveal links show extra text, or another passage's contents, instead of moving to a new passage ([Twine Cookbook](https://twinery.org/cookbook/passagesinpassages/chapbook/chapbook_passagesinpassages.html) [P]).
- **Undum**: "situations" (rooms/pages) output into a scrolling transcript. — [Undum](https://github.com/idmillington/undum) [P] (README); appending behaviour is per the tutorial game, [secondary].
- **80 Days**: conversational rhythm ("you say something, the game says something back") and small screens keep text from being intimidating ([Ingold 2014](https://www.gamedeveloper.com/business/-i-80-days-i-building-the-perfect-text-adventure-for-mobile) [P]). inkle's GDC 2018 talk on text UX centred on "focus and pacing" ([Game Developer](https://www.gamedeveloper.com/design/video-designing-text-ux-for-effortless-reading) [secondary]).
- **Paged contrast**: ChoiceScript's `*page_break` puts a Next button and continues on the next page, and choices likewise advance to a new page ([CoG commands](https://www.choiceofgames.com/make-your-own-games/important-choicescript-commands-and-techniques/) [P]). In 易次元, a `\h` marker keeps the dialogue text visible when options appear and `\p` clears it, so presentation is configured per canvas (course 139 [P]).
- **Streaming**: inkle streamed story content on demand ([80 Days postmortem](https://www.gamedeveloper.com/business/postmortem-inkle-s-i-80-days-i-) [P]).

### Inferences
- REZICS can make "load next chapter" invisible: append the next unit's text under the previous one, and **prefetch all successors of the current choice point**. Fan-out is ~2.4 on average (Q2), so this is cheap. The UX cost of tiny chapters is therefore solvable.
- The **real costs are authoring, management and per-unit overhead**: IDs, titles, metadata, publishing workflow, comments, paywall and analytics per unit.

---

## Q6. Cost model: expected node inflation under "choices only at chapter end"

### Model (*inference, parameterised by cited data*)
Take a scene authored weave-style with *k* inline choice points. Each has *n* options, and a fraction *r* of options carries option-specific text before rejoining. Under the strict rule:

- Each choice point ends a chapter. The text after each rejoin (gather) starts a new chapter. Each option with its own reply text becomes its own chapter.
- **Units per scene ≈ (k + 1) + k·n·r**, so **≈ 1 + n·r units per choice point**.
- Nested sub-choices recurse, and success/failure outcomes multiply.

| Scenario | n | r | Units per choice point | 100k choice points → units |
|---|---|---|---|---|
| Pure cosmetic options, no reply text (same continuation) | any | 0 | ~1.0 | ~100k |
| Half the options have a reply line | 2.4 | 0.5 | ~2.2 | ~220k |
| Typical weave/Choices pattern: each option has a reply, then rejoins | 2.4 (Choices mean) | ~1 | **~3.4** | **~340k** |
| Fallen London-style: each branch has its own outcome event(s), incl. success/failure | 3.5 | ~1.27 | **~5.5** (observed 45,718 / 8,351) | **~550k** |

- **Worked example**: a Choices chapter has k ≈ 22 and n ≈ 2.4, giving ≈ 1 + 22×3.4 ≈ **75 REZICS chapters per Choices chapter**. A 19-chapter Book 1 becomes ~1,400 units. [own analysis]
- **Density cross-check**:
  - At Sorcery!'s ~1 choice per 100 words, 100k choice points ≈ 10M words.
  - At CoG forum guidance of ~200–400 words per choice, it is 20–40M words.
  - Either way, chapters average ~30–150 words. That is gamebook-section size, and far below a web-novel chapter.
- **Cost categories beyond count**:
  1. **Naming/IDs.** ink's manual says that naming every destination slows writing and discourages small branching.
  2. **Redrafting.** Inserting a flavor choice mid-scene splits one chapter into 3+ units and rewires edges. Weave avoids exactly this.
  3. **Editor scaling.** Map views lag at ~600–1,600 nodes.
  4. **Reader latency.** Mitigated by prefetch and append.
  5. **Per-unit platform overhead.** Titles, comments, monetisation, analytics and review workflows (*inference*).
  6. **Localization context.** Tiny units lose surrounding text. Yarn and Dink keep scene context in one file while still keying lines by ID (*inference*).
  7. **Design pressure.** If small choices are expensive, writers will add fewer of them. ink's manual names this explicitly ("discourage minor branching").

---

## Q7. Middle grounds that keep logic out of text documents but avoid node explosion; designer recommendations

### Takeaway
The proven pattern is **two-level granularity**: a coarse *authoring/content* unit (scene or chapter) holds many fine *runtime-addressable* sub-units with stable IDs, and the logic layer references those IDs. Yarn (node + `#line` IDs incl. options), Dink (ink + line IDs + string tables), ink (knots compiled into containers addressed by name or index path, labelled choices and gathers), articy (nested containers) and 易次元 (segment → canvases, 插播 call/return) all work this way. Designers' consistent advice:
- one unit per scene, with local choices inside it
- merge early; record choices as state rather than structure
- split into a new unit only when a side-branch becomes long or unwieldy

### Cited Findings
- **Yarn line IDs**: every line *and option* can carry a stable `#line:` ID. The compiler splits a script into code plus a per-language string table, and the runtime delivers only line IDs. — [Yarn Localisation & Assets](https://docs.yarnspinner.dev/yarn-spinner-for-unity/assets-and-localization) [P]; sibling note Q1.
- **Dink** (ink dialogue pipeline): every line gets a unique stable ID (e.g. `#id:TheTavern_Line_AbCd`). It emits a runtime JSON per line ID plus per-language strings files, and the ID is the key for audio and localisation. — [Dink article](https://wildwinter.medium.com/dink-a-dialogue-pipeline-for-ink-5020894752ee) [P]; [GitHub](https://github.com/wildwinter/dink) [P]
- **ink addressing**: paths reference knots, stitches, gathers and *named* choices by name, and other content by index. Labelled choices/gathers give visit-count state without separate nodes. — [ink JSON runtime format](https://github.com/inkle/ink/blob/master/Documentation/ink_JSON_runtime_format.md) [P]; label example in [Ingold 2020](https://medium.com/game-writing-guide/introduction-to-ink-3e6c224865f8) [P]
- **Call/return instead of new continuation units**:
  - ink tunnels: "do this story, then continue from here" ([WritingWithInk](https://github.com/inkle/ink/blob/master/Documentation/WritingWithInk.md) [P])
  - 易次元 剧情插播: return to the calling canvas after the target segment (course 69 [P])
  - 橙光 呼叫子剧情 [community]
  - Ren'Py `call` ([Ren'Py docs](https://www.renpy.org/doc/html/menus.html) [P])
- **Storylets and variants without inline conditionals**: Yarn 3 line groups pick one salient line among siblings, and node groups pick among same-named nodes by `when:` conditions ([Yarn 3 blog](https://yarnspinner.dev/blog/yarn-spinner-30-what-to-expect/) [P]). Fallen London keeps choice (branch) text separate from event text, with 711,515 words in branches ([Failbetter](https://community.failbettergames.com/t/fallen-london-has-almost-4-5-milion-words/22727) [P]).
- **Hierarchy to tame counts**:
  - articy nesting runs from broad top-level sections down to single dialogue lines. It has no hard rules and exists for when one flow layer gets cumbersome ([articy](https://www.articy.com/en/tutorial-nesting/) [P]).
  - 易次元 剧情目录 folders (course 46 [P]).
  - 闪艺 章节 → 剧情 ([P](https://www.3000.com/course.html)).
- **Granularity recommendations**:
  - ink: use a knot per scene and stitches for events within it; weave for local branching; divert to a new stitch when nesting gets unwieldy ([WritingWithInk](https://github.com/inkle/ink/blob/master/Documentation/WritingWithInk.md) [P]).
  - Yarn: nest options, but jump to a new node when it becomes hard to read ([P](https://docs.yarnspinner.dev/2.5/getting-started/writing-in-yarn/lines-nodes-and-options)).
  - Episode: merge early and use flags ([P/secondary](https://episodesupport.zendesk.com/hc/en-us/articles/115004056594-Complex-Branching)).
  - CoG: use `*fake_choice` and stats-based delayed branching ([P](https://www.choiceofgames.com/2017/12/a-taxonomy-of-choices-establishing-character/)).
  - Ashwell: gauntlet and branch-and-bottleneck as manageable shapes ([P](https://heterogenoustasks.wordpress.com/2015/01/26/standard-patterns-in-choice-based-games/)).
  - 橙光 journal: short branches return to the main line ([P](https://www.66rpg.com/t_107/nuBRkeVd34.shtml)).

### Recommended mitigations for Narrata/REZICS (*inference*)
1. **Separate "chapter" (REZICS document) from "node" (Narrata address).**
   - A chapter document is an ordered list of text blocks with stable block IDs: plain prose, no logic, no option syntax.
   - Narrata nodes reference `chapterId#blockId` or block ranges.
   - The graph (choice → response block → rejoin block) lives entirely in Narrata.
   - This keeps 1 scene = 1 chapter and makes local choices ~0.0 extra chapters.
2. **Two choice kinds in the engine model.**
   - *Local choice*: options plus optional response blocks, then rejoin at a block in the same chapter. It may set state.
   - *Branch choice*: options lead to other chapters.
   - The evidence says ~95%+ of choices are local.
3. **Keep option labels and short replies as keyed strings, not chapters.** Store them as REZICS "snippets" or block IDs inside the parent chapter document, following the Yarn/Dink string-table model. Option text is narrative text, so it belongs to REZICS; which option exists and when is logic, so it belongs to Narrata.
4. **Call/return (tunnel/插播) for reusable or longer side-scenes.** The side chapter returns to the caller's continuation anchor, so no continuation chapter has to be split off.
5. **Variants by selection, not inline conditionals.** Alternative blocks are siblings with IDs, and Narrata picks one by condition or saliency, as in Yarn line groups and storylets.
6. **If the strict rule is kept anyway:**
   - Budget **~3.4× choice points** (up to ~5.5× with outcome variants): ~340k–550k units for 100k choice points, not 100k–300k.
   - Make micro-chapters a lightweight content type (no title, paywall or comments).
   - Group them into scene bundles that are fetched and prefetched together.
   - Render them appended as a continuous flow.
   - Auto-generate IDs for anonymous response and rejoin units, as ink does for unnamed weave content.
   - Provide hierarchical or text-first authoring, never a flat node map.

---

## Summary (≤600 words)

**What the evidence says.** "Passage = node, choices only at passage end" is a real and long-lived model. Gamebook sections (FF 400, Lone Wolf 350), inkle's own inklewriter and early ink, Twine passages, Fallen London storylets, 闪艺 (one screen = one 剧情) and 口袋方舟 (end-of-section 抛出选项) all use it. It is **not** current best practice for the *majority* of choices. Nearly every widely used tool later added an in-unit choice that rejoins automatically: ink weave, ChoiceScript `*choice`/`*fake_choice`, Yarn options, Ren'Py `menu`, Episode choice blocks, 易次元 分支选项 (officially recommended), 橙光 文字选项, and even Twine's reveal/append macros. inkle states the reasons directly:
- node-per-choice forces naming every destination, which slows writing and discourages minor branching
- goto links leave loose ends
- inserting a choice means taking the graph apart

Weave was "the biggest change" in early ink and pivotal to 80 Days.

Field data agrees. In six Choices Book 1s (103 chapters, 2,287 choice points; community wiki, own parse):
- ~22 choices per chapter
- ~2.4 options per choice
- ~55% of choice points have no stat effect on any option
- only ~3% route to distinct paths

Nearly all choices rejoin within the chapter, a few lines later. Sorcery! put a small choice every ~100 words, some purely flavor.

**Main cost.** Each local choice under the strict rule produces a pre-choice unit, a unit per option with its own reply, and a new continuation unit. That gives **≈1 + n·r chapters per choice point**:
- ~1.0 if options have no reply text
- **~3.4 for the typical pattern** (n ≈ 2.4, a reply per option)
- **~5.5 with success/failure outcomes** (Fallen London's observed events per root storylet: 45,718/8,351)

For 100k choice points, expect **~340k–550k chapters**, not 100k–300k. One Choices-style chapter becomes ~75 micro-chapters averaging ~30–150 words.

There is no precedent at this scale. Fallen London has 29,390 branches after 15 years, and 80 Days has 12,000 nodes. Node-map editors lag at ~600–1,600 passages. Secondary costs:
- an ID per unit
- painful redrafting (adding a flavor choice splits a chapter)
- per-unit platform overhead
- localization context loss
- design pressure toward fewer choices

Reader UX is *not* a blocker. The ink web template, Jonah, Harlowe/Chapbook reveals and chat fiction render many small nodes as one continuous flow, and 80 Days streamed content on demand.

**Recommended mitigations (logic stays out of text):**
1. **Decouple content unit from engine node.** A REZICS chapter is a logic-free sequence of text blocks with stable IDs. Narrata addresses `chapter#block` and owns all choice/rejoin structure, as in Yarn `#line` IDs, Dink and ink's index-addressed weave content.
2. **Model two choice kinds.** *Local* choices rejoin inside the chapter and only set state; *branch* choices load another chapter.
3. **Store option labels and short replies as keyed snippets/blocks**, not chapters.
4. **Use call/return** (ink tunnels, 易次元 插播) for side-scenes so the continuation needn't be split.
5. **Select variants by condition or saliency** (Yarn line/node groups) instead of inline conditionals.
6. **If the strict rule stays:**
   - budget 3.4–5.5× choice points
   - make micro-chapters lightweight
   - bundle and prefetch successors
   - render appended
   - auto-ID anonymous units
   - give authors hierarchical or text-first tools, never a flat map
