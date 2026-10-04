# REZICS chapter scale: can one Book hold 100k–300k chapters? (local codebase only)

**Scope.** I read the local repo `D:/rezics-repos/rezics-next` at `058c74fe` (2026-10-04, clean tree) and changed nothing in it. All paths below are relative to that repo root unless marked otherwise.

**Tags:**
- **IMPL**: code I read.
- **TEST**: an executable test, with what it asserts.
- **MEAS**: a recorded measurement, in docs or a commit/brief.
- **DOC**: a design statement or plan.
- **EST**: my own arithmetic or estimate. It is not measured.

**Proposed model under review:**
- Each Narrata passage is a REZICS chapter: an occurrence in a Book Structure that targets a chapter Work, which has per-language Content variants.
- Choices appear only at the end of a chapter.
- The REZICS Structure is **not** the narrative graph.

---

## 0. Verdict

**Not feasible as-is.**
- **Structure format:** one Book Structure admits up to 1,048,576 placements, so 300k placements are representable.
- **Interactive edits:** these were made O(1) per change (G-1014).
- **Everything around the Structure** either hard-fails, or does work that grows with the chapter count on every edit or read:
  - Content search has a **global** cap of 20,000 public units.
  - The chapter-label index fully rebuilds on every head change, with a quadratic scan.
  - Each composition change triggers a global Discovery full build.
  - Each chapter read runs two generation-wide neighbour sorts.
  - Stage, import, refresh and seal are capped at 4,096. Restore is capped at 32.
  - Writing goes through one TDB2 writer, at about 4 graph commits per chapter-language.
- **Semantics:** previous/next, numbering, Continue and spoiler gating all assume linear Structure order, which is wrong for a branching book.
- **The "10,000-chapter" evidence:** it is 10,000 placements that all target **one** chapter Work, split into three volume Structures of ≤3,500. It does not show 10,000 chapter Works with published content (§1).

---

## Q1. The 10,000 / 1,000-chapter tests: what was measured

### Takeaway
- Every "10,000-chapter" test builds **10,000 placements whose `target` is the same single chapter Work**.
- Every test except G1014 splits them into **3 volume Book Structures of ≤3,500**, which stays under the 4,096 stage cap.
- No test creates distinct chapter Works, drafts, publications or search units at that scale.
- Assertions cover per-request **work counters and correctness**, not latency. The latencies are console logs and are not retained in docs.

### Tests (TEST)
| Test | File:lines | Build method | What is asserted | Resource class |
|---|---|---|---|---|
| G1014 write cost | `tests/qa/integration/g-1014-structure-cost.test.ts:19-114` | One Book. Seeds 100 placements via one stage page (`:53-67`), then grows to 100/1,000/10,000 through `POST /v1/compositions/{id}/changes` in **16-op batches** (`:86-94`, about 619 changes). All inserts target `chapter.work` (`:38-40`, `:68-69`). | Per single `insert last` at each scale (`:101-106`): `placementsWritten ≤33`, `segmentsWritten ≤2`, `rebalanced ≤32`, `pagesRead ≤20`, `pagesWritten ≤10`, segment rows ≤3. Final `placementCount 10000` (`:112`). Target authorization runs once per seal/activate (`:63`, `:67`). ms is only logged (`:110`). | `ordinary`: 2 GiB container, `-Xmx512m`, 480 s budget, 450 s test timeout (`scripts/qa/resource-classes.ts:4-9`) |
| G1021 reading cost | `tests/qa/integration/g-1021-reading-cost.test.ts:26-139` | A work-composition root with 3 volume Books (`:73-80`). Each volume is staged in 256-entry pages, then sealed and activated (`:84-105`). **One shared chapter Work** per scale (`:72`, `:95`). | Build <600 s (`:109`). Full chooser traversal returns count+3 items (`:120-127`). Numbered, CJK and saved-position seeks are correct (`:128-135`). It records ms, graph calls/rows/bytes and object reads/bytes, but **asserts no numeric cap**. | `large-tmpfs`: 7 GiB, `-Xmx1536m`, 600 s (`resource-classes.ts:10-15`, `:36-37`) |
| G1021 unit | `services/main/tests/g-1021-reading-order.test.ts:66-91` | Immutable tree fixture at 100/1k/10k | Each distant page/rank seek uses **≤8 object reads and 2 graph queries** (`:82-83`) | n/a |
| G1022 label index | `tests/qa/integration/g-1022-reading-cost.test.ts:30-234` | Same 3-volume, single-target topology (`:111-147`) | Index results equal a legacy scan baseline (`:186-192`). Partial "indexing" state before the backfill (`:152-154`). Interrupted backfill resumes at 1,000 (`:205-230`). Fuseki heap is probed. `buildMs <600 s`. | `large-tmpfs` |
| G954 large journey | `tests/qa/integration/g-954-reading-position.test.ts:129-269` | 3 volumes × **3,500** staged chapters = 10,503 occurrences. **One chapter Work** (`:168`, `:187-197`). | Exact full traversal order (`:241-248`). Multilingual and full-width seeks (`:249-255`). Whole journey <600 s (`:234`). | `large-tmpfs` (`resource-classes.ts:46`) |
| 1,000-chapter cases | `tests/qa/integration/g-847-reading-position.test.ts:36`, `g-954-reading-position.test.ts:23`, `services/main/tests/g-940-summary.integration.test.ts:92-107` | 1,000 placements | G940: a collection summary uses 3 graph queries, and its median is <750 ms and <3×small+150 ms (`:87`, `:106-107`) | |
| Chapter read cap | `services/main/tests/work-contents.integration.test.ts:102-107` | A tiny Book | `GET /v1/chapters/{id}` uses **≤64 graph queries**. This was not measured at scale. | |

### Measurements (MEAS)
- **G954 timings.** These are from the archived briefs, read through `git show 74be30c0`, `7d8726f2` and `2541700d` of `docs/goals/tasks/G-1014.md`, `G-1021.md` and `G-1022.md`. The briefs were removed in `5e35ca0c` "Archive 282 closed briefs".
  - Before G-1014, the 10,503-occurrence API build took **391 s**, about 27 occurrence writes/s.
  - After G-1014, setup fell from 122 s to **47 s**, with 257 s in total.
  - After G-1021, the case fell from 278 s to **81 s**.
  - G-1022 then replaced a full label scan with a maintained index.
- **Catalogue writes (G-1038)**, `docs/storage/workload-budgets.md:204-230`. Disk-backed Jena 6.2.0; durable commits and allocated TDB2 KiB per Work:

  | Path | 100 Works | 1,000 Works | 10,000 Works | Commits |
  |---|---:|---:|---:|---:|
  | Ordinary publication + classification (ms/Work) | 3,058 | 3,422 | **2,965** | 5 |
  | Ordinary path, allocated TDB2 KiB/Work | 5,272 | 8,012 | **11,868** | |
  | Compound catalogue (ms/Work) | | | 792 | 1 |
  | Bulk catalogue, 128 items (ms/Work) | | | 28.5 | 1 per 128 |

  - G-1032 query attempts on the 1k and 10k restores **timed out at 420 s** and are unqualified (`:246-251`).
- **Discovery full build (G-1063):** 1.90 / 13.59 / **135.15 s** at 100 / 1k / 10k Works, under a 5-minute budget at 10k (`workload-budgets.md:184-192`).
- **Qualified host mix (OPS05/SEARCH18):** 100,000 Works and 10,000 public MatchUnits. Read p95 was 366–447 ms; edit p95 was up to about 1 s. The 20,000-unit search scale, the 500M-entity scale and sustained profiles are **not qualified** (`workload-budgets.md:82-107`).
- **Hardware.** All of the above ran on local Docker QA stacks with the resource classes above (`docs/testing/complexity.md:132-138`). The planned production topology is "one 16-core/64 GB host and one 12-core/32 GB host, not evidence of fit" (`workload-budgets.md:5-7`).
- **Harness caps.** The load harness rejects more than 10,000 Works (`scripts/load/cli.ts:66-69`; `scripts/load/corpus.ts:134-136`). Fixture build/restore has a hard **600 s** ceiling (`workload-budgets.md:29-36`).

---

## Q2. Hard limits that a 300k-chapter book hits

| Limit (IMPL) | Value | Where | 300k implication (EST) |
|---|---|---|---|
| Placements per Structure | 1,048,576 | `services/main/src/modules/structure/format.ts:16`, enforced `change.ts:1005` | OK: 300k is 29% of the cap |
| Nesting depth | 16 overall; Book `maxDepth 2` (group → chapter) | `format.ts:18`; `structure/profiles.ts:102`; README `:98-106` | OK |
| Page entries / bytes / tree levels | 256 / 256 KiB / 6 | `format.ts:19-21` | OK. Height is about 3 at 300k: ≥1,172 record leaves |
| Order segment size / key length | 32 members / 32 chars | `format.ts:22-24` | ≥9,375 segments (about 18k after splits), each 5 triples |
| Operations per change | **16**, all of one kind | `change.ts:28`, `:128-130` | ≥**18,750** change commands to place 300k pre-existing Works |
| Focus subjects per command | **100** (the Jena command ceiling) | `change.ts:1357-1360` | Caps a change at about 16 inserts |
| Chapter-Work creation | **1 chapter per command** (`POST /v1/works/{id}/chapters`, one insert op plus a new Work) | `services/main/src/routes/studio.ts:170-241` | **300,000 commands = 300,000 Structure revisions**, strictly serial per Book (expected-head CAS, `:180`, `:224`) |
| Stage / import / refresh records | **4,096** | `format.ts:26`; `stage.ts:18`, `:145-147`, `:184-186`; `refresh.ts:56-60`, `:125-127`, `:255`; README `:63-72`, `:84-85` | A single 300k Structure **cannot** be staged, imported or refreshed. Needs ≥**74** Structures of ≤4,096 |
| Stage projection batch | 30 records/graph command | `format.ts:28`; `change.ts:1860-1866` | 10,000 graph commands to activate 300k across the volumes; 137 per 4,096 |
| Whole-Structure seal | **4,096** placements, else `CompositionTooLarge` | `change.ts:29-30`, `:1484-1487` | A 300k Structure **cannot be sealed**, so seal-based export is impossible (`modules/export/readers.ts:208-232`). Needs ≥74 seals |
| Restore (route path) | **32** (`segmentMembers`); staged 4,096 | `change.ts:1686-1694`, `:1834-1836`; route `routes/compositions.ts:497-515` | Cannot restore a 300k Structure |
| Seal/activation target checks | One Access check per **distinct** target | `stage.ts:211-214`; `change.ts:1499-1503` | 4,096 Access checks per volume seal. Tests used one target, so this was **never exercised** |
| Content draft text | 65,536 UTF-16 units (3×65,536 bytes); document ≤1,000,000 B; revision body ≤1 MiB | `content-publication/draft.ts:77-81`; `services/content/src/document-body.ts:19`; `services/content/src/core.ts:123` | Per chapter, fine |
| Content exact batch | 64 revisions / 4 MiB per SQL | `services/content/src/core.ts:124-125`, `:758-778` | Internal only (§Q5) |
| **Public content search population** | **20,000 eligible variants, global** | `modules/work/search-readiness.ts:11`; enforced `content-publication/search.ts:75-80`, `:140-141` | **300k × L languages ≥ 15× over**. See Q3 |
| Contents numbering | ≤200 top-level groups | `work-contents/read-contract.ts:51-52`; `read.ts:154-156` | 74 volumes fit |
| Rate limit, `write` family | anonymous 10, new 30, **member 120**, trusted 600, service 6,000 per minute | `modules/rate-limit/budgets.ts:13-19`; routes `rate-limit/routes.ts:478-489`, `:494-498`, `:752` | 4 writes per chapter-language (create, draft, publish, eligibility) = **1.2M writes**: member **6.9 days**, trusted **33 h**, service **3.3 h** of pure budget |
| Quotas | Catalogue intake has 3 pending slots; Space-creation quota; generic ledger | `modules/catalogue-intake/store.ts:98-117`; `access/baseline-quota.ts:35`; `modules/quota/schema.ts` | **No per-Book or per-maintainer chapter quota found** |
| Graph layout nodes | 2,000 | `modules/graph-layout/schema.ts:8`, `:28` | See Q7 |

**Command arithmetic for 300k chapters, one language (EST):**
- Via the Studio route: 300k chapter commands, plus 300k drafts, 300k publications and 300k eligibility decisions. That is **1.2M API writes** and **about 1.2M Jena commits**: 4 per chapter-language, counting the relay's MatchUnit.
- At G-1038's ~0.6 s per ordinary commit (2,965 ms / 5 commits), that is **≈200 h ≈ 8.3 days** of single-writer time. This ignores label-index and Discovery work, and growth effects.
- Each extra language adds 3 writes and 3 commits per chapter.

---

## Q3. Operations that grow with chapters/placements

| Operation | Cost | Where | Verdict at 300k |
|---|---|---|---|
| **Occurrence-label index**, which backs chapter search in the reading-position chooser | **Every Structure head change re-queues a full rebuild of that generation.** The clear phase deletes old entries 64 per command; the index phase projects 64 per command. Each index batch re-walks the generation from the start and skips `offset` rows (OFFSET scan). | Native `infra/jena/command-module/src/main/java/com/rezics/jena/OccurrenceLabelIndex.java:65-82` (queue resets), `:91-99` (head change triggers queue), `:130-140` (clear), `:141-163` (OFFSET scan); worker `services/main/src/modules/structure/label-index-backfill.ts:30-32`, `:139-152` | **Breaks.** Per edit at 300k: about 4,688 clear plus 4,688 index commands, and Σ offset ≈ 64·4688²/2 ≈ **7×10⁸ row visits**. That is about 900× the 10k case (7.9×10⁵). Every receipt and outbox batch is retained. A new edit restarts the rebuild, so a book under active authoring never converges, and search reports `indexing` or a partial result. |
| **Content (chapter-body) search readiness** | Each joint graph/Content position triggers a full inventory audit that counts every public variant, head and unit. It fails above 20,000. | `content-publication/search.ts:64-145`; `prepareContentSearch` `:149-161`; health `routes/health.ts:78-103`; mapped to HTTP 422 at `routes/problems.ts:491-493` | **Hard break.** Once the whole platform has >20,000 eligible variants, `/health/search-ready` returns 503 and every public content phrase query returns 422. One 300k book alone does this. The comment at `search-readiness.ts:10` says the cap is "no longer a population admission limit", but `search.ts:140` still enforces it. |
| **Discovery refresh** | A `structure.command` whose action is not create, seal or measures is classified **`scope`**, which forces a full build. Full builds scan **every `schema:CreativeWork`**, including chapter Works, and filter chapters out per candidate. | `modules/discovery/effects.ts:34-37`, `:157-165`; README `:51-58` ("even a harmless label/move inside `composition.change` uses that conservative fallback"); `discovery/source.ts:216-243` | **Breaks.** Every chapter insert or move triggers a global rebuild (135 s at 10k Works, measured). 300k chapter Works add 300k candidates: roughly +68 min per build if per-candidate cost matches (EST, unmeasured). |
| `GET /v1/chapters` previous/next | Two `neighbor()` queries per read. Each BINDs a CONCAT position string for **every placement in the generation**, then FILTERs and sorts for `LIMIT 21`. | `work-contents/read.ts:283-320` (called at `:437-438`); README `:103-105` ("compare one composite position across the generation") | **O(n) engine work ×2 per read.** Fixed round trips, but not fixed engine work. Unmeasured at scale: G1021/1022 measured reading positions, not chapter reads. |
| Contents numbering (`topGroups`) | `SUM(?members)` over all order segments under each top group | `work-contents/read.ts:143-162` | O(segments) ≈ O(n/16) per Contents page or chapter read when chapters sit in groups. Bounded only by the 200-group LIMIT. |
| Contents listing, composition page, chooser pages, numbered/CJK seek | Order-tree seeks, ≤100 per page | `structure/read.ts:206-306`; `reading-position/traversal.ts:12-19`, `:130-143`; G1021 unit test | **Bounded.** The graph-only fallback is O(n) (OFFSET at `traversal.ts:124-127`, segment sums at `:196-217`) but is not used when `structureObjects` is configured. |
| Continue / nextUnread | ≤8 exact page reads; exact unread count only for ≤20 placements | `structure/reading-order.ts:6-12`; `continue/chapters.ts:23-48`, `:185-188`; `continue/contract.ts:23-26` | **Bounded, but linear-order semantics** |
| Reading-position "Mine" (furthest point read) | Loops over **all** of the reader's completed occurrences in the continuity, 50 per page, with graph lookups. No cap except the deadline. | `reading-position/chooser-position.ts:31-42` | O(reader's completed chapters) per chooser request |
| Spoiler withholding (wiki revelations) | Exact location comparison (bounded), plus `privateSnapshot` = `string_agg` + md5 over **all** of the reader's progress, sessions and library rows, run on the initial read and on each fence | `reading-position/boundary.ts:202-209`, `:229-232`, `:269-277`; `reading-position/store.ts:42-54` | O(reader history) per request. It also assumes a linear prefix ("boundary ≥ revealed", `boundary.ts:150-155`), which is wrong for branches. |
| Whole-composition inventory | Capped at 10,000 occurrences | `reading-position/boundary.ts:36-38`, `:99`, `:146`; `contract.ts:7-9` | Diagnostic-only; no production caller found |
| Seal | Reads the whole revision; capped at 4,096 | `change.ts:1484-1503` | **Impossible** |
| Restore | ≤32 (route) | `change.ts:1686-1694` | **Impossible** |
| Export | A Structure exports only through a seal, paged 100 at a time | `modules/export/readers.ts:208-232` | **Impossible** without a seal. Per-reader library export (`library-export/bundle.ts:49-90`) is cursor-paged and fine. |
| Structure revision storage | **Not a full manifest per revision.** A copy-on-write B+tree: each change rewrites the changed leaves plus their ancestors and a small root (`tree.ts:6-8`, `:101-125`; `format.ts:182-206`). No page GC exists. | `structure/tree.ts`; `format.ts` | Page sizes from computed JSON (EST): record ≈452 B (leaf ≈113 KiB), order and interior ≈181 B (≈45 KiB). One 300k state ≈ **190 MB** of leaves. One insert writes about 6–7 pages ≈ **≤340 KB**. 300k single-chapter creates retain ≈50–100 GB; 18,750 16-op changes retain ≈9–17 GB. |
| Publication | Per chapter-language: publish, then eligibility, then relay MatchUnit | §Q4 | O(chapters × languages) commits |

---

## Q4. Per-chapter fixed costs and footprint

### Creating one chapter Work (IMPL)
`POST /v1/works/{book}/chapters` is in `routes/studio.ts:170-241`; the web client is at `apps/web/features/studio/content-api.ts:360-396`. It is a single admitted command that does all of the following:
- **Access (PostgreSQL):**
  - admission register, claim and outcome (`structure/change-admitted.ts:117-143`);
  - `work:edit` authorization;
  - target reads.
- **One Jena write.** It deletes and reinserts the head and placement count, and inserts the following (`change.ts:1270-1345`):
  - **Placement:** about 10 triples. These are type, occurrence, generation, role, segment, key, label, item and selection mode (`change.ts:1071-1105`).
  - **Occurrence `ListItem`:** 3 triples (`:1227-1228`).
  - **Segment updates:** 5 triples per segment (`:1107-1112`).
  - **Chapter Work:** 6 triples. **Main Version:** 4 triples (`:1283-1288`).
  - **Revision anchors:** 2 × 9 triples (`:1289-1297`).
  - **StructureRevision:** about 14 triples (`:594-611`).
  - **Receipt:** about 20 triples (`:1320-1329`).
  - **Outbox:** about 13 triples, carrying two events: `structure.command` and `studio.chapter.create` (`:391-403`).
- **Immutable objects (S3/RustFS):** two Work-component manifests (`:1271-1282`), plus the Structure pages (§Q3).
- **Total:** roughly **90 quads** (EST).

### Making it readable, per language (IMPL)
The web Publish path is `apps/web/features/studio/content-api.ts:146-226`; `publishLatest` is at `:449-490`.

1. **`POST /v1/content-drafts`:** Access admission, then a Content PostgreSQL compare-and-swap that writes the variant and revision rows plus an outbox event. It requires that the target is a CreativeWork with a Main Version (`content-publication/draft.ts:53-68`).
2. **`POST /v1/content-publications`:**
   - Access admission;
   - a Content preparation pin;
   - **one Jena command**: the variant publication head, the decision, the receipt and the outbox;
   - then reconcile (`content-publication/README.md:3-28`).
3. **`POST /v1/content-search-eligibility`:**
   - Access admission with scope `content:search-eligibility:<resource>` and permission `work:edit` (`eligibility-admitted.ts:25-28`);
   - **one Jena command**.
   - **This is a per-publication decision.** It names the exact `publicationDecision` and an expected prior eligibility head, and attests `rightsBasis: 'original-contribution'` and `disclosure: 'public'` (`content-api.ts:196-207`; README `:40-55`).
   - **Who decides.** The README calls the decider "the reviewer", but Studio has the same editor self-attest immediately after publishing. There is no human review queue.
   - **Republishing** creates a new decision, so it needs a new eligibility decision as well.
4. **Content relay (asynchronous): one more Jena command.** It writes:
   - a public **MatchUnit** that carries the **full chapter text** as `rv:searchBody` (`content-publication/relay.ts:153-161`), and Lucene indexes that text;
   - a projection anchor, a receipt and an outbox entry (`:143-178`).
5. **Occurrence-label projection.** Each placement gets a text entity and state rows. This is redone on every head change (§Q3).

**Eligibility is mandatory for reading.**
- The chapter body is served only when a **public** eligibility decision exists (`work-contents/read.ts:99-117`; `canReadTarget` `:77-97`). The comment there reads: "Current public Content publication is the only body source, including for a private reader."
- **Consequence for gamebooks:** every readable passage is publicly full-text searchable. That spoils branch content, and there is no private or locked passage path.

### Footprint for 300k chapters, one language (EST)
- **Jena:** about 90 + 35 (publish) + 25 (eligibility) + 50 (MatchUnit) + label entity ≈ **≈200 quads/chapter**, so **≈60M quads**. Each extra language adds about 110. Add 300k chapter bodies duplicated into TDB2 plus Lucene (about 2.7 GB at 9 KB/chapter, a typical CJK web-novel chapter).
- **TDB2 allocation.** Scaling G-1038's measured 11,868 KiB/Work, the ordinary path at 10k background, gives **≈3.3 TiB of allocated blocks before compaction**. The compound path's 4,000 KiB gives ≈1.1 TiB; bulk's 135.5 KiB gives ≈39 GiB. These are allocated-block figures, explicitly not logical size (`workload-budgets.md:227-230`). Bytes per fact are still unmeasured (`scripts/load/budget.ts:10-15`).
- **Against the documented budgets:**
  - The qualified corpus is **100,000 Works and 10,000 MatchUnits** (`workload-budgets.md:84-86`). One 300k book is **3× all qualified Works** and **30× the qualified search units**.
  - The global planning target is 500M entities (`workload-budgets.md:5-7`), so in principle it is a small fraction. **No path is qualified** above 10k.

---

## Q5. Reader path: the cost of one chapter read

`readChapter` is at `work-contents/read.ts:378-461`. Counts are for a public reader whose chapter sits directly under the Book (IMPL; counts are EST):
- **Graph queries:**
  1. occurrence → structure link;
  2. header;
  3. Work basis (`work/read-header.ts:21-81`);
  4. composition-for-Main;
  5. header;
  6. revision row plus `canReadTarget` (1–2) inside `readCompositionPage` (`structure/read.ts:120-171`);
  7. `selectedContent`;
  8. placement order key;
  9. `neighbor` previous (1 query, plus `canReadTarget` and `selectedContent` per candidate, up to 20);
  10. `neighbor` next (same);
  11. `topGroups`;
  12. basis fence;
  13. `selectedContent` again, then `canReadTarget` again (`:443-445`).

  That is **≈20 graph queries** on the happy path. The worst case adds up to 2×20×3 for undisclosed neighbours. The test cap is ≤64 (`work-contents.integration.test.ts:102-107`).
- **Immutable pages:**
  - the manifest;
  - a record-tree lookup (3 pages);
  - two order-tree `countBefore` descents for the ordinal (6 pages) (`structure/read.ts:158-196`; `tree.ts:175-188`).

  That is **≈10 S3 GETs, ≈475 KB** of JSON that is parsed and TypeBox-validated on every read (EST). **No object cache found** (`infrastructure/immutable-objects.ts`).
- **Content:** one `readExactBatch([revision])`, a single SQL statement (`read.ts:406-408`).
- **Batch body read.**
  - **Public API: none.** `GET /v1/chapters/{id}` and `GET /v1/content-revisions/{revision}` are single-item.
  - **Internally:** `ContentCore.readExactBatch` takes **≤64 revisions / ≤4 MiB in one statement** (`services/content/src/core.ts:124-125`, `:758-778`). `canonicalChapterWorks` resolves ≤24 Works in one query (`structure/chapter-work.ts:4-23`).
- **Prefetching N candidate next chapters:**
  - **The rule.** "Fixed maximum of storage/service round trips for each admitted interactive operation" (`workload-budgets.md:60-71`).
  - **Client-side:** N independent `GET /v1/chapters` calls each satisfy the rule, but cost N × (≈20 queries + 2 O(n) neighbour sorts + ≈475 KB of pages).
  - **A new batch endpoint is feasible within a fixed bound.** For example, ≤8 occurrences per request, which fits 8×~60 KB under 4 MiB. It would use VALUES-batched placement, publication and disclosure lookups, plus one `readExactBatch`, and **no neighbour computation**, because Narrata supplies the successors.
  - **What it needs:** a new cost contract and test.

---

## Q6. Anything lighter than a chapter Work?

**Not implemented.**
- **Book chapters.** A Book `chapter` must target a Work, or the catalog types `schema:Book`/`schema:DigitalDocument` (`structure/profiles.ts:93-105`, `:227-235`). `selection` is `follow-context` or `fixed-revision`, which pins a **whole** Content revision; `fixed-realm` is rejected by commands (`format.ts:64-68`; `change.ts:173-198`, `:1051-1069`).
- **Other targets.** No placement targets a variant, a block or a fragment. Collection `member` can target an occurrence, a realization, a release or a resource (`collection/structure-profile.ts:16-31`), but it carries no body.
- **Drafts.** Drafts require a CreativeWork with a Main Version (`content-publication/draft.ts:53-68`), even though `content.variant.resource_id` is free text with no foreign key (`services/content/migrations/001_core.sql:11-23`).
- **Member replies.** These are the only Work-free bodies, ≤8 KiB (`content-publication/reply-draft.ts:17-76`), and they are not Structure targets.
- **`packages/document`:**
  - Every block, table row, cell and opaque inline has a required `attrs.id`, 1–256 chars, unique per snapshot (`packages/document/src/schema.ts:11`, `:154-158`; `src/index.ts:92-103`).
  - Preservation (DOC, `README.md:63-69`): IDs survive moves and edits, copies get new IDs, split and merge each keep one, and "precise external references also pin the document revision".
  - That preservation is an editor convention (`withDocumentIds` `index.ts:169-194`). No service enforces it.
- **Revision-pinned block references.** `BlockSelector{blockId, cellId?, range}` and cross-revision `resolveBlock` exist only as protocol/library code (`packages/wiki-toolkit/protocol/locator.ts:13-15`, `:34-41`, `:102-215`). Only tests use them. Comments use a revision plus a `TextQuoteSelector`, not block IDs (`services/content/src/comments.ts:146-180`).
- **Transclusion.** Embeds are whole revisions only: 16 direct, 64 closure, depth 8, 4 MiB (`services/content/src/embed.ts:2-3`).
- **Link marks.** The `link` mark has a free `href`. There is no typed link to a Work or occurrence.
- **The docs disagree with the implementation (DOC).** `docs/contracts/work-and-release.md:72-83` says "chapters, arcs and visual-novel routes are **occurrences by default**, not Works". The implemented Book path creates a Work per chapter (`routes/studio.ts:214-226`).
- **Interactive fiction in docs.** No gamebook, choice or hypertext-chapter design exists anywhere in the docs. Interactive fiction appears only as an IFDB/external market (`docs/product/markets-and-growth.md:58`, `:71`).

---

## Q7. Graph visualization in the web app

- **No graph library.** None of the 16 non-`node_modules` `package.json` files depends on cytoscape, sigma, graphology, d3-force/hierarchy, @xyflow/reactflow, @antv/g6, elkjs, dagre, cosmos, vis-network, force-graph or mermaid. I re-checked by grep. The only chart library is `recharts` (`packages/ui/package.json:30`), which pulls in non-graph d3 modules.
- **No graph view in `apps/web`.**
  - `features/relationships/` is social follow/membership, not a graph.
  - Relations render as lists (`apps/web/features/work-levels/connections.tsx`, `relation-rows.ts`).
  - Nothing calls `/v1/graph/queries` or `/v1/graph-layouts`.
  - The about-site only has hand-drawn SVG vignettes of 4–5 nodes (`apps/about/src/illustrations/LineVignette.tsx`, `WikiGrowth.tsx`).
- **Graph-layout module (backend only).** It stores saved view state: node positions (`resource`, `x`, `y`, `group?`, `pinned`) and display groups. It stores **no edges and runs no layout algorithm** (`services/main/src/modules/graph-layout/schema.ts:16-36`; routes `routes/graph-layouts.ts`; table `services/content/migrations/140_graph_layout.sql`).
- **The 2,000-node limit.**
  - **Confirmed:** `MAX_GRAPH_LAYOUT_NODES = 2_000`, `MAX_GRAPH_LAYOUT_GROUPS = 200` (`schema.ts:8-9`, `:28`, `:35`).
  - **No documented reason.** The only text is the cost contract "a fixed number of Access and Content statements for a body of at most 2,000 nodes and 200 groups" (`graph-layout/README.md:17-21`). It was introduced in `9a61261c` without a rationale. The test only compares 1 and 200 positions.
  - **Implicit ceiling (EST).** Bodies are Content revisions capped at 1 MiB, and a node entry is about 110 B, so about 9.5k nodes is the absolute maximum. 300k positions would need about 33 MB.
- **Relation graph API.**
  - `POST /v1/graph/queries` is **one hop**, with 64 edges per page plus a cursor (`modules/graph-query/schema.ts:4-16`). Multi-hop is "prospective"; unanchored closure is not admitted (`docs/contracts/relationship-graph.md:24-28`).
  - REZICS relations are not Narrata's graph anyway.
- **What rendering a 100k–300k-node narrative graph needs.** None of this exists today:
  - a Narrata-side API for graph tiles, neighbourhoods and clusters;
  - a WebGL renderer, such as sigma.js+graphology or cosmos/cosmograph;
  - layout run offline or in a worker, with positions stored by Narrata rather than REZICS graph-layout;
  - level of detail and aggregation, for example by REZICS volume or Narrata scene.

---

## 8. What breaks first, and what would need to change

### Order of failure as a book grows (EST)
**Hard limits:**
1. **4,096 per Structure.** Staged bulk import, refresh, seal and seal-based export all stop here. Restore stops at **32**.
2. **20,000 eligible public variants platform-wide.** Content search readiness returns 503 and phrase queries return 422. A 300k book crosses this at about 6–7% of its chapters, or sooner if the platform already holds other content.

**Degrades continuously (soft):**
3. **Label-index rebuild per edit** (quadratic).
4. **Global Discovery full build per composition change.**
5. **Two O(n) neighbour sorts per chapter read**, plus about 475 KB of tree pages per read.
6. **Single-writer throughput.** About 1.2M commits ≈ 1 week, plus about 50–100 GB of retained Structure pages and possibly TB-scale TDB2 allocation before compaction.

### Changes needed
| # | Change | Why |
|---|---|---|
| 1 | Replace the global `MAX_PUBLIC_UNITS` audit in `content-publication/search.ts` with incremental qualification, **and** decouple "readable" from "public search eligible" | Removes the hard 20k cap. Allows unlisted/spoiler-safe passages. |
| 2 | Delta-maintain the occurrence-label index: index only the changed placements per revision, and use a keyset checkpoint instead of OFFSET. Or disable it for non-linear books. | Removes the quadratic per-edit rebuild. |
| 3 | Give `composition.change` precise owner receipt effects (bounded membership delta), and keep chapter Works out of the Discovery candidate scan | Stops global rebuilds on authoring. |
| 4 | Add a "non-linear" Book profile that turns off neighbour/previous/next, numbering, unread counts, "Mine = furthest", and prefix spoiler gating. Let Narrata supply successors and visibility. | Correctness, and removes O(n) reads. |
| 5 | A batch chapter command: N chapter Works plus placements plus drafts per command, ≤16 to match `MAX_OPERATIONS` and the 100-focus ceiling. Batch publish+eligibility, or a validated bulk path like catalogue bulk (28.5 ms/Work ⇒ about 2.4 h for 300k). | 300k single commands ≈ 8 days. |
| 6 | Streamed/paged stage, seal and restore builders above 4,096, or a deliberate **volume sharding** rule (≥74 Books of ≤4,096 under a work-composition, as every 10k test did) | The caps in Q2. |
| 7 | A bounded batch chapter-body read endpoint (§Q5) and an immutable-page cache | Prefetching candidate passages. |
| 8 | Graph view: add a renderer, plus a Narrata-owned graph and layout service. Do not use graph-layout (2,000-node cap and 1 MiB body). | Q7 |
| 9 | Reconsider granularity | See below. |

**Granularity options:**
- Use the documented model of passages as **occurrences, not Works** (`work-and-release.md:72-83`). That needs Content variants keyed by occurrence and drafts without a Work.
- Or use coarser REZICS chapters (Narrata scenes), with passages addressed by block `attrs.id` plus a pinned revision, through a service-level `BlockSelector` that does not exist yet.

### Unknowns
- Actual per-request latencies of G1014/G1021/G1022: they are console-only and not retained.
- Native Jena engine time for `neighbor()` at large n.
- Logical TDB2 bytes per fact.
- Whether Discovery's per-candidate cost for filtered chapter Works matches the 13.5 ms/Work average.
- Real-world Access cost of 4,096 distinct-target checks per seal.
