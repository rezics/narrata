# Separating Narrative Logic from Concrete Expression: Evidence, Precedents, Boundary Design, Counter-Arguments

Research date: 2026-10-04. Scope: tests the Narrata thesis that "the narrative engine should be fully separated from media and 'real' content like text; media/text are other products", and proposes where the boundary should sit.
Source-quality convention: "(fetched)" means the primary page was read. "(snippet)" means only a search-result snippet or summary was seen, so treat it as lower confidence. Vendor and fan-wiki sources are flagged.

---

## Q1. Object-based media and adaptive/branching video: how do BBC R&D, 2-IMMERSE, and Netflix separate narrative structure from media objects?

### Takeaway
Every serious object-based or branching-video system converged on the same runtime split. A production-agnostic **narrative engine/reasoner** works over a story graph and variables. It emits **references and playlists**, and a separate **representation/composition layer** picks and renders concrete media. The hardest problems were never the runtime split. They were authoring, writing, and production continuity. Netflix shut its system down in 2025, calling the technology "limiting".

### Cited Findings
**Academic framing (Ursu et al., ACM IMX 2020, York/BBC-adjacent; fetched)**
- The paper proposes a "generic (i.e. production-independent) framework for OBM storytelling" with three levels: conceptual, technological, and aesthetic. Its abstractions have "one-to-one operational counterparts" in the authoring toolkit Cutting Room. It was validated with the interactive film *What is Love?*, which over 900 people saw at Mediale 2018 and 94 evaluated by questionnaire. — [Ursu, Smith, Concannon, Hook, "Authoring Interactive Fictional Stories in Object-Based Media", ACM IMX 2020, pp.127-137](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf) ([DOI](https://doi.org/10.1145/3391614.3393654))
- Architecture: "a **narrative engine**, able to operationalise the logic of each particular story in each particular viewing; a **composition engine**, able to aggregate the media assets into the continuous audio-visual stream, as instructed by the narrative engine; the **repository of media assets**." — [Ursu et al. 2020](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf)
- "The most important statement made by this architecture is the **factoring out of the story logic** from the other aspects related to its production and distribution." The narrative engine "dynamically compiles the corresponding version of the story… in the form of **playlists** – i.e. descriptions of the way the media assets are to be composited." The engine goes as far ahead as it can and "pauses where it encounters a decision in the story logic that depends on data not yet provided." — [Ursu et al. 2020](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf)
- "The narrative engine is a **production-agnostic component** and therefore reusable in the development of any OBM production." — [Ursu et al. 2020](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf)
- Atomic objects carry "a unique reference to an element of content and various **metadata annotations**, describing its content and **narrative functions**". There are also "'empty' atomic objects, with no reference to content, which can be used in the design of the story structure or in script writing". "The more refined the atomic elements are, the higher the responsiveness of the production." Conditions refer to object properties such as duration, metadata annotations, interaction variables, and internal variables such as whether an object has been played. — [Ursu et al. 2020](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf)
- Structures: conditional fork, selection group, and conditional layered structure (parallel layers with trigger synchronisation), plus an interactive object that binds an interaction variable or falls back to a default. "Reasoning with the story logic… at viewing time is, essentially, a constraint solving problem." — [Ursu et al. 2020](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf)
- On prior practice: earlier interactive films' logic was "hard-coded in algorithms… programmers are required to translate between creative producers and complex programming languages", which "introduces a huge overhead". — [Ursu et al. 2020](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf)
- StoryFormer was "completed a year later, in 2018, and was used in the production of BBC Click 1000". It is "less expressive… more or less limited to branching structures, expressed through variables and interactive elements", but "illustrates how OBM could be incorporated in broadcasting production workflows". — [Ursu et al. 2020](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf)

**BBC R&D StoryKit / StoryPlayer / OBM data model (fetched)**
- StoryKit consists of StoryPlayer (playback), StoryFormer (graphical authoring), and StoryShooter (on-set media/logging). It ran 2017–2020 and produced Click1000, HDMAdventure (*His Dark Materials*), and *Make Along: Origami Jumping Frog* (2020, the first experience on the new data model). "The data model is tightly specified in the JSON schema language" and is "a shared format… used by all the tools in StoryKit". — [Thomas Preece, StoryKit Part 1, 2020-10-03](https://thomaspreece.com/2020/10/03/storykit-part-1-2-background-playback/)
- StoryFormer used React/Redux/GraphQL. Its purpose was letting creatives build OBM "without software development experience". — [Preece, StoryKit Part 2, 2020-12-06 (snippet)](https://thomaspreece.com/2020/12/06/storykit-part-2-2-authoring-experiences/)
- **Two reasoners.** "the `StoryReasoner` is responsible for evaluating links between Narrative Elements to determine which is taken… while the `RepresentationReasoner` is responsible for determining which Representation is rendered." Both use "the first … whose conditions evaluate to true" and both read a shared **DataResolver** variable store. Built-in variables include `_day_of_week`, `_portion_of_day`, `_path_history`, location, and `_random_number`. A RenderManager selects Representations and drives per-media Renderers (video/audio/image/text) with a construct→start→complete→destroy lifecycle. It also buffers previous/current/next elements for smooth transitions. Behaviours (e.g., pause) attach to Representations. — [bbc/storyplayer docs/implementation.md](https://github.com/bbc/storyplayer/blob/main/docs/implementation.md)
- StoryPlayer's host interface uses fetchers per schema entity (`storyFetcher`, `narrativeElementFetcher`, `representationCollectionFetcher`, `representationFetcher`, `assetCollectionFetcher`) and a `mediaFetcher` that resolves media URIs to playable URLs. It emits events such as `VARIABLE_CHANGED`, `NARRATIVE_ELEMENT_CHANGED`, `NEXT_ELEMENTS`, and `STORY_END`, and accepts an optional `dataResolver` with custom get/set for external data. — [bbc/storyplayer README](https://github.com/bbc/storyplayer/blob/main/README.md)
- The open-sourced OBM schema "models interactive and personalised media experiences in a simple JSON format" (GPLv3). Entities include narrative_element, representation, representation_collection, and asset collections. It "includes a **production domain that is not required for story modelling or playback**" (shots, scenes). — [bbc/object-based-media-schema](https://github.com/bbc/object-based-media-schema)
- CAKE (Cook-Along Kitchen Experience): "OBM separates components and assembles them as late as possible before they reach the user". Recipe steps (audio, video, subtitles) were rescheduled when the cook fell behind. — [Univ. of Manchester Research IT, 2018-10-31 (snippet)](https://research-it.manchester.ac.uk/news/2018/10/31/cake/)
- Stornaway.io (commercial) and StoryFormer "were developed independently around the same time" and "have been working on interoperability". — [Stornaway (vendor page, snippet)](https://www.stornaway.io/ofcom-highlights-stornaway-io-as-industry-leader-in-new-report-on-object-based-media/)
- BBC News Storyline Ontology (published at purl.org/ontology/storyline) models storylines and events "externally to an article context", meaning story structure is separate from the creative works that express it. The ontology has 6 classes and 12 properties. — [IPTC: BBC ontologies (snippet)](https://iptc.org/thirdparty/bbc-ontologies/); [Dublin Core MSI (snippet)](https://msi.dublincore.org/standards/bbc-ontologies/); [Jeremy Tarling, "Storylines as data in BBC News" (403, snippet only)](https://medium.com/@jeremytarling/storylines-as-data-in-bbc-news-bd92c25cba6b)

**EU 2-IMMERSE (2015–Nov 2018)**
- In this object-based multiscreen platform, components (camera angles, audio, graphics) are "only assembled at the viewer's site". The Distributed Media Application (DMApp) splits into a **Timeline Service** ("controls the temporal composition of all the media items") and a **Layout Service** (spatial orchestration). Deployments included the FA Cup Final 2018 at Wembley and a MotoGP trial evaluated with 93 people. — [IRT 2-IMMERSE page (snippet)](https://www.irt.de/en/research/media-services-and-application/2-immerse); [2immerse.eu "What is a DMApp?" (TLS error, snippet only)](https://2immerse.eu/what-is-a-dmapp/); [CORDIS 687655](https://cordis.europa.eu/project/id/687655/reporting)

**Netflix interactive / Bandersnatch (2018–2025)**
- *Bandersnatch* had 150 minutes of footage in 250 segments with four-character alphanumeric segment codes ("the spine of the process"). Some segments had up to 14 cuts, and the final ingest was about 5 hours. Post-production took 17 weeks, 5–7 weeks longer than usual. Writers started in **Twine** and then moved to Netflix's internal **Branch Manager**, which held flowcharts, embedded script, and cut playback and was later made Final Draft-compatible. — [postPerspective (fetched)](https://postperspective.com/netflixs-black-mirror-bandersnatch-lets-viewers-choose/)
- Netflix built "state tracking" to remember choices that later gate segments. "Preconditions" open and close paths depending on state values. Engineers built "a narrative-specific JSON Graph" and "more than 6,000 lines of new client-side code". — [Hollywood Reporter (snippet)](https://www.hollywoodreporter.com/tv/tv-news/black-mirror-bandersnatch-netflixs-interactive-film-explained-1171486/); [ResetEra thread quoting Variety (snippet, secondary)](https://www.resetera.com/threads/variety-how-netflixs-engineers-created-its-first-interactive-movie-black-mirror-bandersnatch.89806/)
- In Netflix's web player, interactive titles were implemented as pluggable "domains". Each is "a static object containing a state data structure, state reducer, actions, middleware, and a custom API to query state", with "custom playback logic and custom UIs". — [Netflix TechBlog, "Modernizing the Web Playback UI" (403; snippet only)](https://netflixtechblog.com/modernizing-the-web-playback-ui-1ad2f184a5a0)
- Device constraint: apps had to pre-cache two possible paths. At launch the title was unavailable on Chromecast, Apple TV, and some legacy TVs. — [TechRadar (snippet)](https://www.techradar.com/news/netflixs-interactive-bandersnatch-movie-isnt-compatible-with-apple-tv-or-the-windows-10-app)
- Netflix removed its last interactive titles on **2025-05-12**, saying "The technology served its purpose, but is now limiting as we focus on technological efforts in other areas". — [Variety 2025 (paywalled; statement via search)](https://variety.com/2025/tv/news/black-mirror-bandersnatch-removal-netflix-1236392097); [GamesRadar (snippet)](https://www.gamesradar.com/entertainment/sci-fi-shows/netflix-is-removing-its-interactive-specials-very-soon-including-black-mirrors-bandersnatch/)

**Metadata fragility**
- Metadata for personalised AV "may initially be generated at some point during the production process but will then be lost at later stages due to current standards and incomplete software implementations." — [Weller, Bleisteiner, Hufnagel, Iber, "The Future is Meta", arXiv 2407.19590, 2024-07-28](https://arxiv.org/abs/2407.19590)

### Inferences
- The BBC StoryPlayer **StoryReasoner / RepresentationReasoner** split is the closest precedent to Narrata's thesis. Narrative reasoning chooses *which narrative element*. A separate reasoner, using the same variables, chooses *which representation* (device, locale, accessibility). This shows that "representation selection" can be state-dependent and still live outside the narrative core, as long as it cannot feed back into branching.
- Ursu's "empty atomic objects" support a media-free core. A story can be fully structured and tested before any media exists, which suits a gamebook reader that runs on placeholders.
- OBM playlists are an *output intent stream* ("composite these refs in this order/layering"). They are not rendered media, which maps directly onto a Narrata presentation-intent stream.
- Netflix's outcome cuts against a *bespoke, player-embedded* interactive stack (6,000 lines of client code per platform, device pre-cache constraints). It is not evidence against separation as such. Tying the narrative runtime to one player was the costly part.
- Metadata loss (Weller et al.) is the operational risk of separation: if line or asset metadata does not survive every pipeline stage, the boundary breaks. Line metadata needs a single owner and must round-trip.

### Gaps
- No Netflix TechBlog post describing Bandersnatch's state-tracking engine was found. The JSON-graph and 6,000-line figures come via secondary reporting.
- bbc.co.uk/rd could not be fetched, so first-party descriptions of "Instagramification of News", "Make-along", and Click 1000 internals were not verified. The Ofcom OBM report was not read directly.
- 2-IMMERSE timeline document format details were not verified (site TLS error).

---

## Q2. Transmedia storyworlds: does theory support a media-independent storyworld/fabula layer?

### Takeaway
Yes, but with nuance. Narratology has long held that *story* is "independent of the techniques that support it" (Bremond). Current "media-conscious narratology" distinguishes **medium-free** concepts (character, action, setting, causality, motivation), **transmedial** concepts that apply to some media (narrator, focalization, interactivity), and **medium-specific** ones (panel, gutter, speech bubble). Industry practice (Lucasfilm's Holocron) runs a media-independent canon database. Jenkins, however, notes that transmedia works best with tight co-creation, not hand-offs.

### Cited Findings
- Bremond (1964), quoted in a 2023 Palgrave handbook entry: story is "a layer of autonomous significance… which can be isolated from the whole of the message". "The structure of the latter is **independent of the techniques that support it**… it is a story that we follow, and it can be the same story." — [Baroni, Goudmand & Ryan, "Transmedial Narratology and Transmedia Storytelling", Palgrave Handbook of Intermediality, 2023](https://www.marilaur.info/baronigoudmadryan.pdf) ([DOI](https://doi.org/10.1007/978-3-030-91263-5_16-1))
- Narrativity as "a media-transcending cognitive construct". Its basic conditions "reside instead in the representation of characters or objects, the presence of change, and the embedding of these events in a network of explanations involving causality, intentionality, planning". — [Baroni, Goudmand & Ryan 2023](https://www.marilaur.info/baronigoudmadryan.pdf)
- Media-conscious narratology distinguishes "**medium-free concepts** (i.e., semantic concepts that apply to narratives in all media… such as character, action, setting, causality, motivation), **transmedial concepts** (… narrator, focalization, and interactivity…), and **medium-specific concepts**, such as frame, gutter, and speech bubble for comics." — [Baroni, Goudmand & Ryan 2023](https://www.marilaur.info/baronigoudmadryan.pdf)
- Counterweight: "the representation of subjectivity is a **highly media-specific** aspect of narratives" (Thon 2016). Film uses ocularization and auricularization, and comics use "emanata". — [Baroni, Goudmand & Ryan 2023](https://www.marilaur.info/baronigoudmadryan.pdf)
- Klastrup & Tosca define transmedia worlds as "**abstract content systems** from which a repertoire of fictional stories and characters can be actualized or derived across a variety of media forms". Ryan: "Transmedia storytelling is not a serial… What holds these stories together is that they take place in the **same storyworld**." — [Baroni, Goudmand & Ryan 2023](https://www.marilaur.info/baronigoudmadryan.pdf)
- Example: audiences treat animated and live-action Ahsoka Tano as the same character "despite the semiotic plasticity of its setting and characters", which is ontological unity across representations (Thon's use of Walton's "principle of charity"). — [Baroni, Goudmand & Ryan 2023](https://www.marilaur.info/baronigoudmadryan.pdf)
- Ryan & Thon's volume treats "storyworlds as representations that transcend media" while calling for "media-consciousness". — [Storyworlds across Media, U. Nebraska Press, 2014](https://www.nebraskapress.unl.edu/nebraska-paperback/9780803245631/storyworlds-across-media/)
- Jenkins (2007-03-21): transmedia storytelling is "a process where integral elements of a fiction get dispersed systematically across multiple delivery channels for the purpose of creating a unified and coordinated entertainment experience". Transmedia stories rest on "complex fictional worlds" that encourage "an encyclopedic impulse". — [Henry Jenkins, "Transmedia Storytelling 101"](http://henryjenkins.org/blog/2007/03/transmedia_storytelling_101.html)
- **Counterpoint, same source:** it "has so far worked best either in independent projects where **the same artist shapes the story across all of the media** involved or in projects where strong collaboration (or co-creation) is encouraged". Licensing, where later media "remain subordinate to the original master text", is the common but weaker case. — [Jenkins 2007](http://henryjenkins.org/blog/2007/03/transmedia_storytelling_101.html)
- Lucasfilm's Holocron continuity database was created in January 2000 in FileMaker Pro and maintained by Leland Chee. It replaced physical "Star Wars Bible" binders. After 2014 the Story Group dropped the tiered canon for a single continuity, and the tier system was confirmed retired on 2018-09-29 (Matt Martin). — [Wookieepedia, "Holocron continuity database" (fan wiki, secondary)](https://starwars.fandom.com/wiki/Holocron_continuity_database)

### Inferences
- Theory supports a media-free *fabula/storyworld* core: entities, states, events, causality, and choice. The medium-free / transmedial / medium-specific split is a usable design rule:
  - **Medium-free → Narrata core:** entities (characters, places, items), state, events, causal/conditional rules, choices.
  - **Transmedial → core carries an abstract, renderer-neutral form:** speaker/narrator identity, focalization/POV ("whose perspective"), interactivity (choice points, timeouts), and coarse pacing (beats).
  - **Medium-specific → never in core:** panels, camera, typography, VO takes, music stems, lip-sync.
- The Holocron shows canon as a database of entities and facts, separate from any book or film. That is close to the "storyworld package" Narrata could expose. It is a *reference* layer, though, not a runtime one.
- Jenkins' co-creation finding warns that separating *products* (engine vs. content tools) is fine, but separating *teams/workflows* so far that writers cannot see expression in context degrades quality (see Q6).

### Gaps
- The Ryan/Thon book chapters were not read directly; claims come via the 2023 handbook entry.
- No primary Lucasfilm source on Holocron schema or workflow was found. The fan wiki is secondary.

---

## Q3. Localization and text/VO pipelines as proof of separation, and what they say about passing state across the boundary

### Takeaway
Shipping pipelines (Yarn Spinner, ink+Dink, Naninovel, articy:draft X, Cyberpunk's JALI) prove that text and VO are routinely **separated at build/runtime by stable line IDs + string tables + line metadata**, even when written inline. They also show that **text depends on narrative state** (plurals, gender, names, variables). The boundary therefore has to carry *typed arguments*, and grammatical selection belongs in the message layer (MessageFormat 2 / Fluent / Yarn `[select]`), not in story logic. Fragment-splicing (ink glue) is the main way coupling breaks localization and VO.

### Cited Findings
**Yarn Spinner**
- Strings files are CSV with `language, id, text, file, node, lineNumber, lock, comment`. "Only the `language` and `text` columns should be modified by the translator." A separate metadata file holds per-line metadata. The "Builtin Localisation Line Provider fetches assets from a Yarn Project, and provides them to voice-over dialogue views." — [Yarn Spinner docs, In-built Localisation](https://docs.yarnspinner.dev/yarn-spinner-for-unity/assets-and-localization/inbuilt-localisation)
- Built-in replacement markers are `[select]` (e.g., gendered pronouns), `[plural]`, and `[ordinal]` (CLDR plural classes by locale). "**individual lines are replaced depending on the user's locale, but the logic surrounding them is not**." The `character` markup attribute lets presentation strip or style speaker names. — [Yarn Spinner docs, Markup](https://docs.yarnspinner.dev/write-yarn-scripts/advanced-scripting/markup)

**ink (inkle) and its production ecosystem**
- The runtime yields content line by line through `Continue()`, plus `currentTags`, knot tags (`TagsForContentAtPath`), `globalTags`, `currentChoices`, external functions, variable observers, and `state.ToJson()`/`LoadJson()` for saves. Tags are suggested for things like VO filenames and expressions. — [ink RunningYourInk.md](https://github.com/inkle/ink/blob/master/Documentation/RunningYourInk.md)
- Tags "don't show up in the main text flow, but can be read off by the game and used as you see fit". "Oddly for a text-engine, **ink** doesn't have much in the way of string-handling: it's assumed that any string conversion you need to do will be handled by the game code." Weaves make it "easy to break a sentence up and insert additional choices for variety or pacing reasons". — [ink WritingWithInk.md](https://github.com/inkle/ink/blob/master/Documentation/WritingWithInk.md)
- Ink-Localiser (Ian Thomas): "Ink is fully capable of stitching together fragments of sentences", which makes translation hard. "It is almost impossible to splice together different sections of sentences for an actor to say." The tool errors on multiple fragments per line and advises "During runtime, **just use Ink for logic, not for content**." IDs look like `<filename>_<knot>(_<stitch>)_<code>`, written back into source as `#id:` tags, with CSV/JSON/POT/PO export. — [wildwinter/Ink-Localiser](https://github.com/wildwinter/Ink-Localiser)
- Dink (2025, MIT): screenplay-style lines `CHARACTER (qualifier): (direction) Text` inside ink. The compiler adds `#id:xxx` to every line and emits three artifacts: the compiled ink JSON (flow), `*-dink.json` (per-line metadata such as speaker and direction), and `*-strings-[locale].json` (text by ID). It also emits an Excel VO recording script and tracks audio status across Final/Recorded/Scratch/TTS folders. — [wildwinter/dink](https://github.com/wildwinter/dink)
- A shipped Unity+ink game used per-line tags as localization keys (e.g., `Shaw_Chap1_Joy_Line2`) to look up text in a localization database. — [John Nemann, "Localizing Ink with Unity" (snippet)](https://johnnemann.medium.com/localizing-ink-with-unity-42a4cf3590f3)

**Naninovel (Unity VN engine)**
- Localization docs use `# ID` / `; Source text` / translation. Only generic text lines are extracted, and commands stay in source. "When a translated generic line contains inlined commands or expressions, it may be split into multiple text fragments, each mapped to a unique text ID." Changing source text invalidates mappings. — [Naninovel Localization guide](https://naninovel.com/guide/localization)

**articy:draft X**
- Built-in localization view with Excel export/import. VO files can be imported "paired with the corresponding text", and Simulation Mode switches languages and plays attached VO. Unity/Unreal importers include a runtime flow interpreter, and there is a generic JSON export. — [articy Localization basics (snippet)](https://www.articy.com/en/adx_basics_localization/); [articy Unity integration (snippet)](https://www.articy.com/en/downloads/unity/); [articy Unreal integration (snippet)](https://www.articy.com/en/downloads/unreal/)

**Message formatting standards**
- Unicode **MessageFormat 2.0 reached Stable in CLDR 47 / ICU 77 on 2025-03-13**: "the stability guarantees are in place and implementations can finalize their APIs." It supports "grammatical matching (such as plurals or genders)" and custom formatting and selection functions. — [Unicode blog, 2025-03-13](http://blog.unicode.org/2025/03/unicode-cldr-47-release-messageformat-2.html)
- MF2 syntax has `.input`/`.local`/`.match`, placeholders `{$x :function opt=val}`, and markup `{#b}…{/b}`, `{#x /}`. Markup represents "non-language parts of a message, such as inline elements or styling", and attributes "MUST NOT have any effect on the formatted output". Default functions (LDML 48.2 text) include `:string, :number, :integer, :offset, :currency, :percent, :unit, :datetime, :date, :time`. Some may be at draft status in the spec, which was not verified per-function. — [LDML Part 9: MessageFormat](https://www.unicode.org/reports/tr35/tr35-messageFormat.html)
- Fluent's principle is "asymmetric localization": "A simple string in English can map to a complex multi-variant translation in another language." "Translators should be able to use the entire expressive power of their language **without asking developers for permission**." Code passes variables such as `$tabCount`, and terms can carry case and gender variants. — [projectfluent.org](https://projectfluent.org/)
- Rust options include community MF2 crates (`mf2-model`, `mf2-syntax`, `mf2-runtime`; plus `forest-rs/message-format` with ICU4X-backed formatting) and `fluent-rs`. Their maturity was not evaluated. — [docs.rs mf2 (snippet)](https://docs.rs/mf2/latest/mf2/); [forest-rs/message-format PR #29 (snippet)](https://github.com/forest-rs/message-format/pull/29)

**VO, barks, lip-sync**
- Valve's Dynamic Dialog (Elan Ruskin, GDC 2012) matches "hundreds of facts about the world" fuzzily against "thousands of possible lines". It applies "the most specific one it can find, using less-specific ones as fall-backs", and lets writers add special cases and running gags "without forcing programmers to change thousands of lines of code." — [GDC Vault (snippet)](https://gdcvault.com/play/1015317/AI-driven-Dynamic-Dialog-through); [Emily Short summary, 2012-03-16](https://emshort.blog/2012/03/16/gdc-2012-talk-on-dynamic-dialogue/)
- *Hades*: a studio of under 20 people shipped a fully voiced game with 22,000+ lines. Kasavin wrote lines for "as many different possible contexts" as possible and had the game select the right one. The GDC 2021 talk covered how the "massive script was optimized for audio production". — [GDC talk listing (snippet)](https://www.classcentral.com/course/youtube-breathing-life-into-greek-myth-the-dialogue-of-hades-165690); [Game Developer interview (snippet)](https://www.gamedeveloper.com/design/roguelikes-and-narrative-design-with-i-hades-i-creative-director-greg-kasavin); [Game Developer GDC preview, 2021-04-13](https://www.gamedeveloper.com/audio/dive-into-the-dialogue-of-i-hades-i-at-gdc-2021)
- *Cyberpunk 2077* used JALI to generate lip-sync and facial animation **from per-language audio + text** in 10 languages, using G2P models and pronouncing dictionaries. In-line tags adjusted emotional expression within a line. — [JALI SIGGRAPH 2020 talk (ResearchGate, snippet)](https://www.researchgate.net/publication/343764404_JALI-Driven_Expressive_Facial_Animation_and_Multilingual_Speech_in_Cyberpunk_2077); [Engadget (snippet)](https://www.engadget.com/cyberpunk-2077-dialogue-language-lip-sync-artificial-intelligence-dubbing-154523977.html)

### Inferences
- **The mature pattern is "inline authoring, compiled separation".** Writers author text inside the logic script (Yarn, ink, Dink, Naninovel). The compiler then assigns stable line IDs and emits separate artifacts: flow program, string table per locale, and line metadata (speaker, direction, VO status). Narrata does not need writers to author text "elsewhere". It needs its compiler to **extract** text into an external content product keyed by stable IDs.
- **Two kinds of variation must not be confused:**
  - *Narrative variation* (different event, different information, different choice availability) belongs in the core and is visible to save, branching, and replay.
  - *Grammatical/expressive variation* (plural, gender agreement, case, honorifics, name insertion, synonym rotation) belongs in the message layer. The core only passes **typed arguments** (numbers, enums such as `gender: feminine`, entity refs). This is Yarn's "lines are replaced… the logic surrounding them is not" and Fluent's "without asking developers".
- **Fragment splicing is the anti-pattern at the boundary.** Ink glue/mid-line diverts break both localization and VO. Narrata should make the **line (utterance) the atomic unit** that crosses the boundary. Any intra-line variability is expressed as MF2/Fluent selectors over passed arguments, not as core-level text concatenation.
- **Line metadata is first-class boundary data**: speaker EntityId, addressee, line class (dialogue/narration/bark/UI), emotion/delivery hint, source location, lock hash (to detect source-text changes, as Yarn and Naninovel do), and translator comment.
- Valve and Hades show that for **barks**, the core should emit a *semantic concept* ("PlayerHurt", "EnteredRoom{room}") plus facts. A separate response-selection layer picks the line. If the chosen bark must be remembered (e.g., "already said"), the selection is a state change and must be recorded by the core.

### Gaps
- No primary inkle statement on its own localization practice (e.g., for *Heaven's Vault*) was found. inkle's GitHub issue #196 contains only a user question.
- Unreal FText/String Table and Unity Localization string-table docs were not fetched.
- Details of the Hades priority/eligibility system were not verified from the talk itself.

---

## Q4. Presentation-agnostic runtime patterns: MVC/Elm, headless engines, command/intent streams, tags, accessibility renderers, asset referencing

### Takeaway
The successful runtimes expose a **pull/step API that yields semantic units** (line + tags/metadata, choice set, commands), keep **state serializable separately from presentation**, and treat presentation as a **function of state plus an intent stream** (Elm-style). Asset delivery systems (Unity Addressables) show stable logical keys resolved through catalogs to physical, possibly remote, content. Counter-patterns (Ren'Py text tags, Naninovel commands, Twine story formats) show presentation timing and format-specific logic leaking into scripts, with real lock-in costs.

### Cited Findings
- The Elm Architecture: "**Model** — the state of your application; **View** — a way to turn your state into HTML; **Update** — a way to update your state based on messages." — [Elm Guide](https://guide.elm-lang.org/architecture/)
- The ink runtime is wrapped, not inherited from, and talks to the game through external functions and variable observers. Content arrives as lines with tags, and saves are JSON state. — [ink RunningYourInk.md](https://github.com/inkle/ink/blob/master/Documentation/RunningYourInk.md)
- Netflix's web player hosted interactive logic as a "domain" (state, reducer, actions, middleware, query API) beside, not inside, core playback. — [Netflix TechBlog (snippet)](https://netflixtechblog.com/modernizing-the-web-playback-ui-1ad2f184a5a0)
- BBC StoryPlayer emits narrative events (`NARRATIVE_ELEMENT_CHANGED`, `VARIABLE_CHANGED`, `NEXT_ELEMENTS`) to the host and resolves media through injected fetchers. — [bbc/storyplayer README](https://github.com/bbc/storyplayer/blob/main/README.md)
- 2-IMMERSE splits Timeline (temporal composition) from Layout (spatial composition) services. — [IRT (snippet)](https://www.irt.de/en/research/media-services-and-application/2-immerse)
- Emily Short separates **resolution** systems (what outcome occurs) from **performance/reporting** systems (how the result is presented). Performance aims to "Avoid verbatim repetition of the same words, postures, or performances". — [Emily Short, "What does your narrative system need to do?", 2022-04-09](https://emshort.blog/2022/04/09/what-does-your-narrative-system-need-to-do/)
- Unity Addressables: assets are loaded by **address/label keys**, and "Content Catalogs are the data stores Addressables uses to look up an asset's physical location based on the key(s)". With a remote catalog, assets can change on a CDN "without forcing users to reinstall". `GetDownloadSize` supports consent before download. — [Unity Addressables: Content catalogs (snippet)](https://docs.unity3d.com/Packages/com.unity.addressables@1.21/manual/build-content-catalogs.html); [Remote content distribution (snippet)](https://docs.unity3d.com/Packages/com.unity.addressables@2.0/manual/get-started-remote-content.html)
- **Counter-pattern, presentation timing inside text (Ren'Py):** `{w}` waits for a click or N seconds, `{nw}` auto-dismisses (and waits for self-voicing), `{cps}` sets characters per second, and `{fast}` sustains voice from the previous line. — [Ren'Py Text docs (snippet)](https://www.renpy.org/doc/html/text.html)
- **Counter-pattern, format lock-in (Twine):** Twine separates passage data from story formats (Harlowe, SugarCube, Chapbook, Snowman), but "The main differences between story formats come in how they handle macros", so scripts are not portable across formats. — [Twine Cookbook: Story Formats](https://twinery.org/cookbook/introduction/story_formats.html)
- **Hybrid in production:** Failbetter used ink "alongside Naninovel" for *Mask of the Rose*, with ink for narrative logic and text and Naninovel for VN presentation. — [MCV/DEVELOP, 2021-03-11](https://mcvuk.com/development-news/want-your-story-to-drive-your-game-rather-than-vice-versa-inkle-and-failbetter-discuss-the-storytelling-potential-of-the-open-source-ink/)
- Narrata's existing docs already specify a declarative `SceneState` (layers, actors, camera, audio channels, interaction). On load or rewind the core emits `ReconcileScene(target)` instead of replaying "show A, hide B, play C" history. One-off "presentation flourish" is not authoritative. Content policies are `Pinned` / `Compatible` / `LivePresentationOnly`, and the last "must not affect guard, choice or Effect payload". — local: `D:/rezics-repos/narrata/docs/architecture/effects-and-host-state.md`, `D:/rezics-repos/narrata/docs/architecture/program-versioning-and-migration.md`, `D:/rezics-repos/narrata/docs/adr/0007-stage-3-effect-host-boundary.md`

### Inferences
- Narrata's `ReconcileScene` design is the Elm pattern applied to time travel. It is the right answer to "how does presentation survive rewind": presentation is re-derived from state, not replayed from commands. Ren'Py-style inline timing would break this unless timing tokens are purely presentational and non-authoritative.
- A **headless engine** is viable only if the step API yields *semantic* units, not formatted text. Suggested minimum event vocabulary:
  - `Line{line_id, speaker, args, meta}`
  - `ChoiceSet{choice_id[], label_ref, args, availability}`
  - `Beat/Await{kind}` (abstract pacing)
  - `SceneDelta` / `SceneState`
  - `Cue{semantic_id}` (music/sfx intent)
  - `Effect` (external)
- For **accessibility and text-only renderers** (screen reader, gamebook reader, CLI), the same stream suffices if every line or cue has a text-resolvable ContentRef and every visual intent has an optional description ref. This makes "text is just another renderer" concrete.
- **Asset references:** use logical keys (like Addressables addresses) in core and authoring. Resolve through a versioned catalog to content-addressed blobs (hash), and pin the catalog/content revision in saves (Narrata's `Pinned`). Logical key and content hash are both needed. The key gives author stability and the hash gives replay determinism.
- The Twine lesson for Narrata is that composable node packages must not let presentation-specific macros into the portable story package. Keep presentation intents in a namespaced, optional vocabulary that renderers may ignore.

### Gaps
- No primary documentation for a "headless CMS used as a narrative backend" (Contentful/Sanity for interactive stories) was found. This analogy is unsourced.
- No primary source on screen-reader-specific IF/VN renderers was gathered.

---

## Q5. Generative media era (2024–2026): does LLM rendering on top of structured narrative state strengthen the case for a media-free core?

### Takeaway
Yes, strongly. Both commercial practice (Hidden Door) and 2024–2026 research (StoryVerse, WhatELSE, Drama Llama, neuro-symbolic world-state transformations) converge on a **structured symbolic state/plot layer plus an LLM "renderer"**. Benchmarks show LLMs alone drift badly (a best of 42% "survival" after 20 turns). Critiques of Hidden Door show what happens when the structured layer is too thin: the renderer becomes the de facto state.

### Cited Findings
- Hidden Door (shipped 2025): the state is **cards** (characters, locations, objects, plot directions) plus a pre-written arc, and the LLM generates prose and choices. The reviewer notes "I can feel how the LLM prompt is built out of these cards". The critique is that "the story is written only as far as it's been presented to the player": past events aren't tracked as objective truth, pronoun and gender inconsistencies appear, and actions rarely fail. — [Ian Bicking, Hidden Door design review, 2025-08-27](https://ianbicking.org/blog/2025/08/hidden-door-design-review-llm-driven-game.html)
- Hidden Door is described as assembling narrative "from a large library of human-written tropes… adapted and recombined". Separately, its engine layer "is updated with a structured representation of everything in the world… used to make the actual text and assemble the bits of art". — [Arcanum review 2026 (snippet)](https://arcanumrpgs.com/blog/hidden-door-review/); [PC Gamer (snippet; exact attribution unverified)](https://www.pcgamer.com/hidden-door-ai-game-narrative-rpg/)
- StoryVerse (Autodesk Research, 2024-05-17): writer-specified **abstract acts** (goals, prerequisites, placeholders) are compiled by LLM narrative planning into character actions over an explicit world state. "By abstracting the story from the specifics of the world states, it becomes more adaptable." Limitations are long-term coherence and LLM-call latency. — [arXiv 2405.13042](https://arxiv.org/abs/2405.13042)
- WhatELSE (CHI 2025): authors define a **narrative space** through pivot stories, abstract outlines, and variants, and "LLM-based narrative planning… unfold[s] the narrative space into executable game events" (N=12 study). — [arXiv 2502.18641](https://arxiv.org/abs/2502.18641); [Autodesk Research page](https://www.research.autodesk.com/publications/whatelse/)
- Drama Llama (2025-01-15): authored **storylets** with natural-language triggers bound LLM generation, which "could generate coherent and meaningful narratives" (preliminary, 6 authors). — [arXiv 2501.09099](https://arxiv.org/abs/2501.09099)
- Góngora, Chiruzzo, Méndez, Gervás (2026-05-23): in a neuro-symbolic interactive storytelling system, LLMs predict which **pre-programmed world-state transformations** to trigger, which "offer a way to maintain world-state consistency while encouraging players to interact creatively through their written inputs" (Llama 3 70B and Gemini 1.5 Flash, EN/ES, 8 participants). — [arXiv 2605.24719](https://arxiv.org/abs/2605.24719)
- NCP-Bench (ICML 2026, submitted 2026-08-08): over 100 narrative environments, "the best-performing model (GPT-5.2) achieving only 42% survival rate after 20 turns". Fact-conflict rates ran 40–68%. — [arXiv 2608.08160](https://arxiv.org/abs/2608.08160)
- SCORE (2025) adds "Dynamic State Tracking" that "monitors and corrects inconsistencies… using symbolic logic". — [arXiv 2503.23512 (snippet)](https://arxiv.org/html/2503.23512v1)

### Inferences
- LLM-era evidence is the strongest *new* support for Narrata's thesis. A deterministic, replayable core is exactly what generative renderers lack. The boundary should treat an LLM narrator as **one more renderer** that consumes `Line/Beat/SceneState` plus storyworld facts and produces text, image, or voice.
- **Hard rule:** generated output must not become narrative truth unless it re-enters through a validated, recorded channel (Narrata's Recorded Query / Effect ledger). This is the Góngora et al. pattern: the LLM proposes, symbolic rules dispose. It also fixes Hidden Door's "story exists only as presented" failure.
- Generated renderings are non-deterministic. For time travel, either (a) cache or record generated artifacts keyed by `(event_id, renderer, model_version, seed)` as `LivePresentationOnly` content, or (b) accept non-identical re-renders on rewind. Narrata's policy table already gives the vocabulary for this choice.
- The core must expose **richer semantic metadata** than a hand-written-text engine would: entity facts, relationships, intents, beat purpose ("reveal", "complication"), and POV. LLM renderers need *why*, not just *what*. This argues for a schema'd "beat/intent" layer between core state and renderers.

### Gaps
- No primary technical write-up from Hidden Door itself was found. Architecture claims come from reviewers.
- No 2025–2026 studio postmortem quantifying LLM-rendered narrative in a shipped commercial game was found beyond Hidden Door reviews.

---

## Q6. Counter-arguments: writing-first philosophy, authorial control, timing-as-narrative, cost of indirection

### Takeaway
The strongest counter-evidence is about **authoring**, not runtime. Ingold ("the programming sits within the text"), Short (pacing is tuned only when seen with art and UI), Ursu et al. (writing was the hardest part, and production forced design simplification), and Jenkins (co-creation beats hand-off) all say that prose, logic, and presentation co-evolve. Timing (VN pacing, cinematic sequencing, Bandersnatch's 10-second choice window) is sometimes narrative. Indirection costs (IDs, metadata loss, format lock-in, bespoke players) are real. Mature teams resolve this with **inline authoring plus compiled separation** and **integrated preview**, not strict separation at authoring time.

### Cited Findings
- Jon Ingold (inkle): ink should be "a tool for humans to write things for other humans". "**The programming sits within the text, not the text sitting within the programming**." — [MCV/DEVELOP, 2021-03-11](https://mcvuk.com/development-news/want-your-story-to-drive-your-game-rather-than-vice-versa-inkle-and-failbetter-discuss-the-storytelling-potential-of-the-open-source-ink/)
- Emily Short (Failbetter, on *Mask of the Rose*): "Ink offers enough coding affordances that we could build that behaviour within our scripts, rather than needing to call out to external code". "That's the point where we can see how the lines play in context with the art and UI. **That's the moment to make pacing changes**." — [MCV/DEVELOP, 2021-03-11](https://mcvuk.com/development-news/want-your-story-to-drive-your-game-rather-than-vice-versa-inkle-and-failbetter-discuss-the-storytelling-potential-of-the-open-source-ink/)
- Ingold, as attributed in search results: writing in games is often "very clunky and clumsy… That's the fault of the tools". The exact page (MCV "Why Inkle is sharing the tech" vs. Haywire interview) was not verified. — [MCV/DEVELOP (snippet)](https://mcvuk.com/development-news/why-inkle-is-sharing-the-tech-behind-80-days-and-sorcery/); [Haywire, "Wordplay: Jon Ingold" (snippet)](https://haywiremag.com/features/wordplay-jon-ingold/)
- ink's weave design explicitly supports breaking "a sentence up and insert[ing] additional choices for variety or pacing reasons", so prose granularity is a narrative tool. — [ink WritingWithInk.md](https://github.com/inkle/ink/blob/master/Documentation/WritingWithInk.md)
- Ursu et al.'s expert analysis: "Writing was the most difficult part of the process", balancing "a reason for the viewer to interact" against "allowing the story to flow naturally". "There was a divide between the intentions regarding the structure… agreed in design workshops and the ones resulted after the content had [been] produced, which simplified significantly the design." "A more iterative development process was found to be necessary, but difficult to implement… due to the requirements for continuity." Production also revealed "the need for shooting grammars", and "despite a generous production budget, major simplifications to the story concept still had to be made". — [Ursu et al. 2020](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf)
- Ursu et al.'s framing: "the tight interdependency between form and technology: the development of compelling productions require appropriate authoring tools, while the development of appropriate tools require compelling forms to respond to." — [Ursu et al. 2020](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf)
- *Bandersnatch* cost: shooting took 2–3 weeks longer and post 5–7 weeks longer. Viewers had a **10-second choice window**, with automatic selection on timeout, so timing was part of the interaction design. — [postPerspective](https://postperspective.com/netflixs-black-mirror-bandersnatch-lets-viewers-choose/)
- Ren'Py exposes pacing (`{w}`, `{nw}`, `{cps}`, voice sustain) inside dialogue strings, showing VN authors treat rhythm as part of the line. — [Ren'Py Text docs (snippet)](https://www.renpy.org/doc/html/text.html)
- Representation of subjectivity is "highly media-specific" (Thon). — [Baroni, Goudmand & Ryan 2023](https://www.marilaur.info/baronigoudmadryan.pdf)
- Jenkins: transmedia works best when "the same artist shapes the story across all of the media" or with strong co-creation. — [Jenkins 2007](http://henryjenkins.org/blog/2007/03/transmedia_storytelling_101.html)
- Indirection costs: Naninovel and Yarn mappings break when source text changes, so lock hashes and regeneration are needed. Metadata gets "lost at later stages". Twine macro lock-in. — [Naninovel](https://naninovel.com/guide/localization); [Yarn Spinner](https://docs.yarnspinner.dev/yarn-spinner-for-unity/assets-and-localization/inbuilt-localisation); [Weller et al. 2024](https://arxiv.org/abs/2407.19590); [Twine Cookbook](https://twinery.org/cookbook/introduction/story_formats.html)
- A tool vendor's comparison: "Ink is text-first, so you do not get a visual overview of your dialogue tree", versus articy's visual flow editor. This is vendor marketing, low weight. — [storyflow-editor blog (vendor, snippet)](https://storyflow-editor.com/blog/best-narrative-design-tools-for-game-developers-2025/)

### Inferences
- Narrata's thesis survives if restated as "**runtime- and product-level** separation". It fails if read as "**authoring-level** separation". Writers must be able to author text inline with logic and preview with representative presentation. The engine product can still be media-free if the *authoring product* (e.g., the gamebook editor in REZICS) compiles inline text out to the content product.
- **Timing is narrative only at the interaction level.** Timed choices, default choices, and "await acknowledgment" belong in the core as **inputs and abstract beats** (a timeout is a recorded input event, which keeps determinism). Micro-rhythm (cps, pauses inside a line, music sync) belongs in presentation metadata on the line, not in core state.
- Mitigating indirection cost:
  - **Stable IDs auto-assigned by the compiler and written back into source** (Ink-Localiser, Dink, Yarn `#line:`).
  - **Lock hashes** to detect stale translations and VO.
  - **One owner for line metadata**, round-tripped through every export.
  - **A reference renderer** (the gamebook reader) maintained in lockstep so authors always see expression.
- The "cost of a bespoke player" (Netflix) argues that the presentation side needs a stable, small, documented intent protocol that several hosts can implement. That is cheaper than one rich integrated player.

### Gaps
- No primary Emily Short essay specifically arguing that prose and logic must co-evolve was located. The quotes above are interview statements.
- No published articy-vs-ink post-mortem from a studio (as opposed to vendor comparisons) was found.
- The rhythm and timing of Hades barks was not verified from primary material.

---

## Q7. Product strategy: companies that split engine, content tools, and media pipelines into separate products, and how it affected adoption

### Takeaway
The most-adopted narrative middleware ships a **free/open core runtime + compiler** with **engine integrations** and optional paid presentation add-ons (Yarn Spinner, ink). Visual authoring tools (articy:draft X, Arcweave) separate **authoring product → export/JSON/API → engine plugins**. Broadcaster OBM (BBC) open-sourced its data model and player and kept the authoring tool internal. Netflix's vertically integrated approach ended. Presentation-specific runtimes (Twine formats, Naninovel) win on the presentation side but bind content to them.

### Cited Findings
- Yarn Spinner (2026-01-05): "The core will always be free and open source. Everything that makes Yarn Spinner work is in the open source version." Paid add-ons (Dialogue Wheel, Speech Bubbles, Visual Novel Kit, Text Animator integration) are "extras on top, not essentials behind a paywall". Engine integrations cover Unity, Godot, and Unreal (native Unreal launching 2026). Shipped titles include *Night in the Woods*, *DREDGE*, *A Short Hike*, and *Lost in Random*, and the post claims "thousands" of shipped games. — [Yarn Spinner, "Yarn Spinner in 2026"](https://yarnspinner.dev/blog/yarn-spinner-in-2026)
- Yarn Spinner for Unity 3.2 shipped March 2026, Godot (GDScript) was leaving beta, and Unreal was still in beta as of the September 2026 update. — [Yarn Spinner Monthly Update Sep '26 (snippet)](https://yarnspinner.dev/blog/monthly_sep_26)
- ink is open source. Failbetter adopted it "very early" and paired it with Naninovel for presentation, so engine and presentation came from different vendors. — [MCV/DEVELOP 2021](https://mcvuk.com/development-news/want-your-story-to-drive-your-game-rather-than-vice-versa-inkle-and-failbetter-discuss-the-storytelling-potential-of-the-open-source-ink/); ink is described as "almost like a middleware" — [MCV/DEVELOP, "Why Inkle is sharing the tech" (snippet)](https://mcvuk.com/development-news/why-inkle-is-sharing-the-tech-behind-80-days-and-sorcery/)
- articy:draft X: a standalone authoring and localization/VO product, Unity/Unreal importers with runtime interpreters, generic JSON export, and a free tier on itch.io. — [articy Unity (snippet)](https://www.articy.com/en/downloads/unity/); [articy:draft X FREE (snippet)](https://articy-software.itch.io/articydraft-x-free)
- Arcweave is web-based and collaborative, with open-source Unity/Unreal/Godot plugins and a web API for Team workspaces (runtime story updates via project JSON). Its self-reported user count of "over 20,000 devs" is a vendor claim from 2024. — [Arcweave web API (snippet)](https://arcweave.com/docs/1.0/api); [Arcweave on X (vendor claim)](https://x.com/arcweave/status/1840688388799627365)
- BBC open-sourced the OBM data model (GPLv3) and StoryPlayer. StoryFormer was BBC-internal, and Stornaway (commercial) pursued interoperability. — [bbc/object-based-media-schema](https://github.com/bbc/object-based-media-schema); [bbc/storyplayer](https://github.com/bbc/storyplayer); [Ursu et al. 2020](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf); [Stornaway (vendor, snippet)](https://www.stornaway.io/ofcom-highlights-stornaway-io-as-industry-leader-in-new-report-on-object-based-media/)
- Netflix built an internal tool (Branch Manager) and a player-embedded runtime, then retired interactive titles in May 2025. — [postPerspective](https://postperspective.com/netflixs-black-mirror-bandersnatch-lets-viewers-choose/); [GamesRadar (snippet)](https://www.gamesradar.com/entertainment/sci-fi-shows/netflix-is-removing-its-interactive-specials-very-soon-including-black-mirrors-bandersnatch/)
- Twine separates data from runtime "story formats", but macro differences cause format lock-in. — [Twine Cookbook](https://twinery.org/cookbook/introduction/story_formats.html)

### Inferences
- Pattern: **open/free core runtime + language** (drives adoption) → **engine/host integrations** (drives reach) → **paid presentation kits and authoring/services** (drives revenue). That is consistent with Narrata as a media-free core and the Rezics gamebook reader and other media tools as separate products.
- Adoption is driven by authoring ergonomics and host integrations, not by purity of separation. Yarn and ink win on writer-friendliness, and articy and Arcweave on visual and collaborative authoring. The core can be media-free, but the *product family* must ship at least one first-class authoring experience and one reference renderer, or the core will be judged by its worst integration.
- Open data models (BBC OBM schema) enabled interoperability attempts (Stornaway), while closed integrated stacks (Netflix) died with their host's strategy. A **published, versioned boundary schema** (events, line metadata, content refs) is itself a strategic asset.

### Gaps
- No adoption metrics independent of vendors (download counts, market share) were found for ink, Yarn, articy, or Arcweave.
- No evidence was found about headless CMS (Contentful/Sanity) adoption for narrative apps.

---

## Q8. Where exactly should Narrata's boundary be? (Synthesis of the above)

### Takeaway
Put the boundary at **semantic, ID-addressed, typed events**, not at "text vs. no text". The core owns structure, state, rules, choices, abstract pacing/interaction timing, and stable content references with typed arguments. Everything that turns those into words, voices, pixels, or sound (including grammatical agreement, locale, rhythm inside a line, asset files, and generative renderers) lives outside. It may read state but must never write it except through recorded inputs. Authoring may stay inline. Separation is enforced at the compiler and runtime artifact level.

### Cited Findings
- The runtime split is proven in BBC StoryPlayer (StoryReasoner vs RepresentationReasoner over a shared DataResolver) and Ursu's OBM architecture (narrative engine → playlist → composition engine). — [bbc/storyplayer implementation](https://github.com/bbc/storyplayer/blob/main/docs/implementation.md); [Ursu et al. 2020](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf)
- The artifact split is proven in Dink (flow JSON + per-line metadata JSON + per-locale strings JSON), Yarn (strings CSV + metadata CSV), and Naninovel (ID'd localization docs, commands in source). — [Dink](https://github.com/wildwinter/dink); [Yarn Spinner](https://docs.yarnspinner.dev/yarn-spinner-for-unity/assets-and-localization/inbuilt-localisation); [Naninovel](https://naninovel.com/guide/localization)
- Grammatical variation in the message layer with typed args is the standard: MF2 (Stable 2025-03-13), Fluent, and Yarn `[select]/[plural]`. — [Unicode](http://blog.unicode.org/2025/03/unicode-cldr-47-release-messageformat-2.html); [Fluent](https://projectfluent.org/); [Yarn markup](https://docs.yarnspinner.dev/write-yarn-scripts/advanced-scripting/markup)
- Medium-free / transmedial / medium-specific concept taxonomy. — [Baroni, Goudmand & Ryan 2023](https://www.marilaur.info/baronigoudmadryan.pdf)
- Symbolic core + LLM renderer in research and practice, with LLM-only drift quantified. — [arXiv 2605.24719](https://arxiv.org/abs/2605.24719); [arXiv 2608.08160](https://arxiv.org/abs/2608.08160); [Bicking 2025](https://ianbicking.org/blog/2025/08/hidden-door-design-review-llm-driven-game.html)
- Narrata already defines `ReconcileScene`, `LivePresentationOnly`, Recorded Query, and REZICS `ContentStructureNode` references. REZICS owns Post body, Portable Text, localization, and authorization. — local: `D:/rezics-repos/narrata/docs/architecture/effects-and-host-state.md`, `D:/rezics-repos/narrata/docs/architecture/program-versioning-and-migration.md`, `D:/rezics-repos/narrata/docs/rezics-gamebook-integration.md`

### Inferences
**Proposed boundary contract.** This is a design inference, not a sourced fact.

1. **What the core emits (the boundary payload):**
   - `Utterance { occurrence_id, content_ref, speaker: EntityId?, addressee?, class: dialogue|narration|bark|system, args: TypedMap, meta: LineMeta }`. Here `args` holds numbers, enums such as gender/plurality, EntityRefs, and booleans: what MF2 `.input` or Fluent `$var` need. `LineMeta` holds delivery/emotion hint, POV/focalization, beat purpose, and an author comment.
   - `ChoiceSet { choices: [{choice_id, label_ref, args, enabled, reason_ref?}], timeout?: {beats|seconds, default_choice_id} }`. The timeout and its result are recorded inputs.
   - `Beat { kind: pause|await_ack|section_break|scene_change, weight }`. This is abstract pacing, with no milliseconds and no cps.
   - `SceneState` (declarative, semantic: actors present, location, mood/tension, active cues as semantic IDs) plus `ReconcileScene` on load and rewind. This already exists.
   - `Concept { concept_id, facts }` for bark/response-rule systems (Valve style). The core emits the concept and the response layer selects. If the selection matters to story state it must be returned as a recorded query.
   - Storyworld facts (entities, relations, knowledge state) queryable read-only by renderers, especially LLM renderers.
2. **What stays out of the core:**
   - Resolved text in any locale, grammatical agreement, and string formatting (MF2/Fluent/Portable Text in REZICS).
   - VO takes and lip-sync (keyed by `content_ref + locale`).
   - Images, video, and music stems (logical asset keys → catalog → content hash).
   - Typography, layout, and in-line rhythm (cps, intra-line pauses).
   - Camera, and generative model calls.
3. **Invariants:**
   - (a) Nothing resolved outside can change guard/choice/effect results unless it re-enters as a Recorded Query (Narrata already states this for `LivePresentationOnly`).
   - (b) The **utterance is the atomic text unit**, so the core never concatenates text fragments. Intra-line variation is MF2/Fluent selection over passed args.
   - (c) *Narrative* variants (different information or consequence) are distinct `content_ref`s chosen by core logic. *Grammatical/stylistic* variants are message-level.
   - (d) Every `content_ref` is stable, compiler-assigned, written back to source, and protected by a source lock hash.
   - (e) Saves pin content revisions according to policy (`Pinned` / `Compatible` / `LivePresentationOnly`).
4. **Authoring hybrid:** let writers write prose inline in Narrata's authoring format (ink/Yarn-style). The compiler extracts it into the REZICS content product and line-metadata tables, so the runtime package contains IDs, not prose. Ship a reference renderer (the gamebook reader) and a "preview with presentation" loop so pacing can be tuned in context, which addresses Short's point.
5. **Thesis verdict:**
   - "Narrative engine separated from media and text **at runtime and as a product**" is well supported by OBM, StoryPlayer, localization pipelines, middleware business models, and LLM-era neuro-symbolic work.
   - "Fully separated, including at authoring time and including timing/pacing" is contradicted by inkle, Failbetter, the Ursu production experience, Jenkins, and VN practice.
   - The core must carry transmedial abstractions (speaker, POV, interactivity, beats), not just raw state. Otherwise every renderer reinvents them and the LLM renderer lacks the "why".

### Gaps
- No published standard exists for a cross-engine "narrative presentation intent" schema. The proposal above is synthesized, not adopted from a spec.
- Narrata-specific performance costs of the indirection (ID lookups, Wasm boundary crossings for the gamebook reader) were not measured here.
