import { expect, test, type Page } from "@playwright/test";
import { mkdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "../../..");
const screenshots = resolve(root, ".temp/goal/runs/vn-player/screenshots");

async function openPlayer(page: Page, width = 1586, height = 992) {
  await page.setViewportSize({ width, height });
  await page.goto("/");
  await expect(page).toHaveTitle("The River Road");
  await expect(page.getByText("The last train is already gone.", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue dialogue" })).toBeEnabled();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  await expect(page.locator(".scene-background")).toBeVisible();
}

async function toChoice(page: Page) {
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("It ends at the old hall.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByRole("button", { name: "Take the narrow passage.", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Cross the courtyard.", exact: true })).toBeVisible();
}

async function toChoiceWithAudioBaseline(page: Page) {
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("It ends at the old hall.", { exact: true })).toBeVisible();
  const playsBeforePreparation = (await metrics(page)).audioEvents.filter(event => event.event === "play").length;
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByRole("button", { name: "Take the narrow passage.", exact: true })).toBeVisible();
  return playsBeforePreparation;
}

async function waitForCandidatePreparation(page: Page) {
  await expect.poll(async () => page.evaluate(() => window.__vnTestHooks?.controller.getSnapshot().candidates?.status)).toBe("complete");
}

async function waitForCandidatePreparationToSettle(page: Page) {
  await expect.poll(async () => page.evaluate(() => window.__vnTestHooks?.controller.getSnapshot().candidates?.status)).toMatch(/^(complete|partial)$/);
}

async function clearPreparedCaches(page: Page) {
  await page.evaluate(async () => { await window.__vnTestHooks?.clearPreparedCaches(); });
}

async function unlockSound(page: Page) {
  await page.getByRole("button", { name: "Turn sound on" }).click();
  await expect(page.getByRole("button", { name: "Turn sound off" })).toBeVisible();
}

async function capture(page: Page, name: string) {
  await mkdir(screenshots, { recursive: true });
  await page.screenshot({ path: join(screenshots, name), fullPage: true });
}

async function recordMetrics(page: Page, name: string) {
  const evidence = await page.evaluate(() => window.__vnTestHooks?.metrics ?? null);
  if (!evidence) return;
  await mkdir(screenshots, { recursive: true });
  await writeFile(join(screenshots, `${name}.json`), `${JSON.stringify(evidence, null, 2)}\n`);
}

test.afterEach(async ({ page }, testInfo) => {
  await recordMetrics(page, testInfo.title.replace(/[^A-Za-z0-9_-]+/g, "-").toLowerCase());
});

async function currentView(page: Page) {
  const view = await page.evaluate(async () => await window.__vnTestHooks?.currentView());
  if (!view) throw new Error("The VN browser probe did not expose a current view.");
  return view;
}

async function metrics(page: Page) {
  const evidence = await page.evaluate(() => window.__vnTestHooks?.metrics);
  if (!evidence) throw new Error("The VN browser probe did not expose evidence.");
  return evidence;
}

async function save(page: Page) {
  const value = await page.evaluate(async () => await window.__vnTestHooks?.save());
  if (typeof value !== "string") throw new Error("The VN browser probe did not export a durable save.");
  return value;
}

async function expectToolbarControlsFit(page: Page, width: number, height: number) {
  const controls = await page.locator(".player-toolbar a, .player-toolbar button").evaluateAll(elements => elements.map(button => {
    const bounds = button.getBoundingClientRect();
    return {
      label: button.getAttribute("aria-label") ?? button.textContent,
      left: bounds.left,
      top: bounds.top,
      right: bounds.right,
      bottom: bounds.bottom,
      height: bounds.height,
      clientWidth: button.clientWidth,
      scrollWidth: button.scrollWidth,
    };
  }));
  expect(controls.length).toBeGreaterThanOrEqual(4);
  expect(controls.some(control => control.label?.toLowerCase().includes("sound"))).toBe(true);
  for (const control of controls) {
    expect(control.left).toBeGreaterThanOrEqual(0);
    expect(control.top).toBeGreaterThanOrEqual(0);
    expect(control.right).toBeLessThanOrEqual(width);
    expect(control.bottom).toBeLessThanOrEqual(height);
    expect(control.height).toBeGreaterThanOrEqual(44);
    expect(control.scrollWidth, `${control.label} content must fit its touch target`).toBeLessThanOrEqual(control.clientWidth + 1);
  }
}

async function closeAndReload(page: Page) {
  await page.evaluate(async () => { await window.__vnTestHooks?.close(); });
  await page.reload();
}

test("desktop first screen is a real host-resolved scene and loads only the entry graph", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await openPlayer(page);
  await expect(page.locator(".speaker-name")).toHaveText("Mira");
  await expect(page.locator("[data-layer='mira']")).toBeVisible();
  await expect(page.locator("[data-layer='tomas']")).toBeVisible();
  await expect(page.locator("[data-background='background/river-night']")).toBeVisible();
  await expect(page.getByText(/artifact:|commit:/i)).toHaveCount(0);
  await expect(page.locator("vite-error-overlay")).toHaveCount(0);

  const view = await currentView(page);
  const evidence = await metrics(page);
  const chunks = evidence.logicalRequests.filter(item => item.kind === "chunk");
  expect(view.view.depth).toBe(0);
  expect(chunks).toHaveLength(1);
  expect(chunks[0]?.phase).toBe("required");
  expect(evidence.candidateQueries).toBe(0);
  expect(evidence.logicalRequests.find(item => item.kind === "manifest")?.bytes).toBeGreaterThan(0);
  expect(evidence.mediaRequests.map(item => item.key)).toEqual(expect.arrayContaining([
    "background/river-night", "character/mira", "character/tomas", "music/river-road", "voice/mira-opening",
  ]));
  expect(evidence.mediaRequests.map(item => item.key)).not.toContain("background/old-hall");
  expect(evidence.mediaRequests.map(item => item.key)).not.toContain("voice/tomas-road");
  expect(evidence.contentRequests.map(item => item.key)).toEqual(expect.arrayContaining(["dialogue.mira.opening", "speaker.mira"]));
  expect(evidence.contentRequests.map(item => item.key)).not.toContain("choice.prompt");
  expect(evidence.logicalRequests.reduce((total, item) => total + item.bytes, 0)).toBeGreaterThan(0);
  expect(evidence.mediaRequests.reduce((total, item) => total + item.bytes, 0)).toBeGreaterThan(0);
  expect(errors).toEqual([]);
  await capture(page, "desktop-dialogue.png");
});

test("Space advances one dialogue safe point without skipping a line", async ({ page }) => {
  await openPlayer(page);
  const before = await currentView(page);
  await page.evaluate(() => { if (document.activeElement instanceof HTMLElement) document.activeElement.blur(); });
  await page.keyboard.press("Space");
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  const after = await currentView(page);
  expect(after.view.depth).toBe(before.view.depth + 1);
  expect(after.view.interaction.kind).toBe("advance");
  expect((await metrics(page)).readerCalls.advance).toBe(1);
});

test("390 by 844 touch stage advances one line and keeps both characters visible", async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2.1875, isMobile: true, hasTouch: true });
  const page = await context.newPage();
  try {
    await openPlayer(page, 390, 844);
    await expect(page.locator(".sound-control .switch")).toBeHidden();
    await expect(page.getByRole("button", { name: "Turn sound on" })).toHaveAttribute("aria-pressed", "false");
    await expectToolbarControlsFit(page, 390, 844);
    const before = await currentView(page);
    await page.touchscreen.tap(195, 430);
    await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
    const after = await currentView(page);
    expect(after.view.depth).toBe(before.view.depth + 1);
    expect(await page.locator("[data-layer='mira']").isVisible()).toBe(true);
    expect(await page.locator("[data-layer='tomas']").isVisible()).toBe(true);
    await expectToolbarControlsFit(page, 390, 844);
    expect(await page.getByRole("button", { name: "Continue dialogue" }).evaluate(button => button.getBoundingClientRect().height)).toBeGreaterThanOrEqual(44);
    await expect(page.locator("[data-background='background/river-night']")).toHaveAttribute("data-variant", "portrait-cover");
    await expect(page.locator("[data-background='background/river-night']")).toHaveJSProperty("naturalWidth", 853);
    await expect(page.locator("[data-background='background/river-night']")).toHaveCSS("object-position", "50% 50%");
    expect(await page.evaluate(() => ({ width: innerWidth, height: innerHeight, scale: devicePixelRatio })))
      .toEqual({ width: 390, height: 844, scale: 2.1875 });
    await capture(page, "mobile-continue.png");
    const highDensity = await page.screenshot({ path: join(screenshots, "mobile-continue-high-density.png"), fullPage: true, scale: "device" });
    expect(highDensity.readUInt32BE(16)).toBe(853);
    expect(highDensity.readUInt32BE(20)).toBe(1846);
    await recordMetrics(page, "mobile-touch");
  } finally {
    await context.close();
  }
});

test("390 by 844 keeps audio-unavailable copy and its retry control inside the screen", async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  const page = await context.newPage();
  try {
    await openPlayer(page, 390, 844);
    await page.evaluate(() => { if (window.__vnTestHooks) window.__vnTestHooks.controls.corruptAudioKey = "voice/tomas-road"; });
    await page.getByRole("button", { name: "Continue dialogue" }).click();
    await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
    await expect(page.getByText("Sound is unavailable. You can continue without it.", { exact: true })).toBeVisible();
    const retrySound = page.getByRole("button", { name: "Retry sound" });
    await expect(retrySound).toBeEnabled();
    await expectToolbarControlsFit(page, 390, 844);
    await page.locator(".player-toolbar").evaluate(toolbar => {
      toolbar.style.letterSpacing = "0.06em";
      toolbar.style.fontSize = "17px";
    });
    await expectToolbarControlsFit(page, 390, 844);

    const status = await page.locator(".audio-status").boundingBox();
    const continueButton = await page.getByRole("button", { name: "Continue dialogue" }).boundingBox();
    const panel = await page.locator(".dialogue-panel").boundingBox();
    expect(status).not.toBeNull();
    expect(continueButton).not.toBeNull();
    expect(panel).not.toBeNull();
    if (!status || !continueButton || !panel) throw new Error("Expected mobile dialogue controls to have visible bounds.");
    expect(status.y).toBeGreaterThanOrEqual(panel.y);
    expect(status.y + status.height).toBeLessThanOrEqual(panel.y + panel.height);
    expect(continueButton.y + continueButton.height).toBeLessThanOrEqual(panel.y + panel.height);
    const overlapsContinue = status.x < continueButton.x + continueButton.width
      && status.x + status.width > continueButton.x
      && status.y < continueButton.y + continueButton.height
      && status.y + status.height > continueButton.y;
    expect(overlapsContinue).toBe(false);
    await capture(page, "mobile-audio-unavailable.png");

    await retrySound.click();
    await expect(page.getByRole("button", { name: "Turn sound off" })).toHaveAttribute("aria-pressed", "true");
    await expectToolbarControlsFit(page, 390, 844);
    expect((await metrics(page)).retries.audio).toBe(1);
  } finally {
    await context.close();
  }
});

test("resize swaps only the host image variant and leaves the pending save cursor intact", async ({ page }) => {
  await openPlayer(page);
  const beforeView = await currentView(page);
  const beforeSave = await save(page);
  const beforeCalls = await metrics(page);

  await page.setViewportSize({ width: 390, height: 844 });
  const background = page.locator("[data-background='background/river-night']");
  await expect(background).toHaveAttribute("data-variant", "portrait-cover");
  await expect(background).toHaveJSProperty("naturalWidth", 853);
  const afterView = await currentView(page);
  expect(afterView.view.cursor).toBe(beforeView.view.cursor);
  expect(afterView.view.depth).toBe(beforeView.view.depth);
  expect(await save(page)).toBe(beforeSave);
  const afterCalls = await metrics(page);
  expect(afterCalls.readerCalls.advance).toBe(beforeCalls.readerCalls.advance);
  expect(afterCalls.readerCalls.choose).toBe(beforeCalls.readerCalls.choose);
  expect(afterCalls.mediaRequests.some(item => item.key === "background/river-night" && item.variant === "portrait-cover")).toBe(true);
});

test("choice branches change complete scene layers, converge, and history can select an earlier point", async ({ page }) => {
  await openPlayer(page);
  await unlockSound(page);
  const playsBeforePreparation = await toChoiceWithAudioBaseline(page);
  await waitForCandidatePreparation(page);
  const preparedMetrics = await metrics(page);
  expect(preparedMetrics.candidateQueries).toBe(1);
  expect(preparedMetrics.logicalRequests.some(item => item.phase === "candidate" && item.kind === "chunk")).toBe(true);
  expect(preparedMetrics.mediaRequests.filter(item => item.key === "background/old-hall")).toHaveLength(1);
  expect(preparedMetrics.mediaDecodes.some(item => item.phase === "candidate" && item.kind === "image" && item.key === "background/old-hall")).toBe(true);
  expect(preparedMetrics.mediaDecodes.some(item => item.phase === "screen" && item.kind === "audio")).toBe(true);
  expect(preparedMetrics.audioEvents.filter(event => event.event === "play")).toHaveLength(playsBeforePreparation);
  await expect(page.locator(".speaker-name")).toHaveCount(0);
  await expect(page.getByText("It ends at the old hall.", { exact: true })).toBeVisible();
  const choiceTypography = await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).evaluate(button => {
    const choice = getComputedStyle(button);
    const prompt = document.querySelector<HTMLElement>(".choice-prompt");
    const promptStyle = prompt ? getComputedStyle(prompt) : undefined;
    const toolbar = document.querySelector<HTMLElement>(".player-toolbar");
    return {
      choiceFamily: choice.fontFamily,
      choiceWeight: choice.fontWeight,
      promptFamily: promptStyle?.fontFamily,
      promptWeight: promptStyle?.fontWeight,
      toolbarSize: toolbar ? getComputedStyle(toolbar).fontSize : "",
    };
  });
  expect(choiceTypography.choiceFamily).toContain("system-ui");
  expect(choiceTypography.choiceWeight).toBe("400");
  expect(choiceTypography.promptFamily).toContain("Georgia");
  expect(choiceTypography.promptWeight).toBe("400");
  expect(choiceTypography.toolbarSize).toBe("19px");
  await capture(page, "desktop-choice.png");

  await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).click();
  await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
  await expect(page.locator("[data-background='background/old-hall']")).toBeVisible();
  await expect(page.locator("[data-layer='mira']")).toHaveCount(1);
  await expect(page.locator("[data-layer='tomas']")).toHaveCount(0);
  const narrow = await currentView(page);
  expect(narrow.view.scene.layers.map(layer => layer.id)).toEqual(["mira"]);
  expect((await metrics(page)).mediaRequests.filter(item => item.key === "background/old-hall")).toHaveLength(1);
  await expect(page.locator("[data-background='background/old-hall']")).toHaveJSProperty("naturalWidth", 1586);

  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("We found the same door from different sides.", { exact: true })).toBeVisible();
  await expect(page.locator("[data-layer='mira']")).toHaveCount(1);
  await expect(page.locator("[data-layer='tomas']")).toHaveCount(1);

  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "History", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "History" })).toBeVisible();
  await page.locator(".history-list button").first().click();
  await expect(page.getByText("The last train is already gone.", { exact: true })).toBeVisible();
  const evidence = await metrics(page);
  expect(evidence.readerCalls.checkout).toBeGreaterThan(0);
  expect(evidence.logicalRequests.filter(item => item.kind === "chunk").length).toBeGreaterThan(1);
});

test("same-revision candidate media is reused only inside its authorization context", async ({ page }) => {
  await openPlayer(page);
  await toChoice(page);
  await waitForCandidatePreparation(page);
  const firstScope = await metrics(page);
  expect(firstScope.mediaRequests.filter(item => item.key === "background/old-hall").map(item => [item.revision, item.contextScope])).toEqual([["1", "demo-public"]]);
  const releasedBeforeSwitch = firstScope.releases;

  await page.evaluate(async () => { await window.__vnTestHooks?.switchContext("reader-two"); });
  await expect(page.getByRole("button", { name: "Take the narrow passage.", exact: true })).toBeEnabled();
  await waitForCandidatePreparationToSettle(page);
  const secondScope = await metrics(page);
  expect(secondScope.mediaRequests.filter(item => item.key === "background/old-hall").map(item => [item.revision, item.contextScope])).toEqual([
    ["1", "demo-public"],
    ["1", "reader-two"],
  ]);
  expect(secondScope.releases).toBeGreaterThan(releasedBeforeSwitch);

  await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).click();
  await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
  expect((await metrics(page)).mediaRequests.filter(item => item.key === "background/old-hall")).toHaveLength(2);
});

test("an optional candidate resource failure leaves an available branch playable", async ({ page }) => {
  await openPlayer(page);
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("It ends at the old hall.", { exact: true })).toBeVisible();
  await page.evaluate(() => {
    if (window.__vnTestHooks) window.__vnTestHooks.controls.failNext = { key: "background/old-hall", delayMs: 20 };
  });
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByRole("button", { name: "Take the narrow passage.", exact: true })).toBeVisible();
  await expect.poll(async () => page.evaluate(() => window.__vnTestHooks?.controller.getSnapshot().candidates?.status)).toBe("partial");
  await expect(page.getByRole("button", { name: "Take the narrow passage.", exact: true })).toBeEnabled();
  expect((await metrics(page)).mediaRequests.filter(item => item.key === "background/old-hall").map(item => item.status)).toEqual(["unavailable"]);

  await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).click();
  await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
  expect((await metrics(page)).mediaRequests.filter(item => item.key === "background/old-hall").map(item => item.status)).toEqual(["unavailable", "available"]);
});

test("a viewport change cancels the old candidate variant and releases its late lease", async ({ page }) => {
  await openPlayer(page);
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("It ends at the old hall.", { exact: true })).toBeVisible();
  await page.evaluate(() => {
    if (window.__vnTestHooks) window.__vnTestHooks.controls.delayLateNext = { key: "background/old-hall", delayMs: 650 };
  });
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByRole("button", { name: "Take the narrow passage.", exact: true })).toBeVisible();
  await expect.poll(async () => (await metrics(page)).mediaRequests.filter(item => item.key === "background/old-hall").length).toBe(1);
  const pendingVariant = (await metrics(page)).mediaRequests.find(item => item.key === "background/old-hall");
  expect(pendingVariant?.variant).toBe("landscape-cover");

  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.locator("[data-background='background/river-night']")).toHaveAttribute("data-variant", "portrait-cover");
  await expect.poll(async () => page.evaluate(() => window.__vnTestHooks?.controller.getSnapshot().candidates?.status)).toMatch(/^(complete|partial)$/);
  await expect.poll(async () => (await metrics(page)).lateDiscards).toBeGreaterThan(0);
  const evidence = await metrics(page);
  expect(evidence.aborts).toBeGreaterThan(0);
  expect(evidence.releases).toBeGreaterThan(0);
  expect(evidence.mediaRequests.some(item => item.key === "background/old-hall" && item.variant === "portrait-cover" && item.status === "available")).toBe(true);

  await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).click();
  await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
  expect((await metrics(page)).mediaRequests.filter(item => item.key === "background/old-hall" && item.variant === "portrait-cover")).toHaveLength(1);
});

test("a choice reopens cold with the same prompt and no retained speaker", async ({ page }) => {
  await openPlayer(page);
  await toChoice(page);
  const beforeSave = await save(page);
  const before = await currentView(page);
  expect(before.view.interaction.kind).toBe("choose");
  await closeAndReload(page);
  await expect(page.getByText("It ends at the old hall.", { exact: true })).toBeVisible();
  await expect(page.locator(".speaker-name")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Take the narrow passage.", exact: true })).toBeEnabled();
  expect(await currentView(page)).toMatchObject({ view: { cursor: before.view.cursor, depth: before.view.depth, interaction: { kind: "choose" } } });
  expect(await save(page)).toBe(beforeSave);
});

test("a dialogue save closes and cold reopens at the same safe point", async ({ page }) => {
  await openPlayer(page);
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  const before = await currentView(page);
  const beforeSave = await save(page);
  await closeAndReload(page);
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  expect(await currentView(page)).toMatchObject({ view: { cursor: before.view.cursor, depth: before.view.depth } });
  expect(await save(page)).toBe(beforeSave);
});

test("sound unlock is a gesture, Keep preserves music, and checkout restarts current audio at zero", async ({ page }) => {
  await openPlayer(page);
  const before = await currentView(page);
  const initialSave = await save(page);
  await page.getByRole("button", { name: "Turn sound on" }).click();
  await expect(page.getByRole("button", { name: "Turn sound off" })).toBeVisible();
  expect((await currentView(page)).view.cursor).toBe(before.view.cursor);
  expect(await save(page)).toBe(initialSave);
  let evidence = await metrics(page);
  const firstMusicPlay = evidence.audioEvents.filter(event => event.event === "play" && event.role === "music");
  const firstVoicePlay = evidence.audioEvents.filter(event => event.event === "play" && event.role === "voice");
  expect(firstMusicPlay).toHaveLength(1);
  expect(firstMusicPlay[0]?.loop).toBe(true);
  expect(firstVoicePlay).toHaveLength(1);
  expect(firstMusicPlay[0]?.offsetSeconds).toBe(0);
  expect(firstVoicePlay[0]?.offsetSeconds).toBe(0);

  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  await expect.poll(async () => (await metrics(page)).audioEvents.filter(event => event.event === "play" && event.role === "voice").length).toBe(2);
  evidence = await metrics(page);
  expect(evidence.audioEvents.filter(event => event.event === "play" && event.role === "music")).toHaveLength(1);
  expect(evidence.audioEvents.filter(event => event.event === "play" && event.role === "voice")).toHaveLength(2);

  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(page.getByText("The last train is already gone.", { exact: true })).toBeVisible();
  await expect.poll(async () => (await metrics(page)).audioEvents.filter(event => event.event === "play" && event.role === "voice").length).toBe(3);
  evidence = await metrics(page);
  expect(evidence.audioEvents.filter(event => event.event === "play" && event.role === "music")).toHaveLength(2);
  expect(evidence.audioEvents.filter(event => event.event === "play" && event.role === "voice")).toHaveLength(3);
  expect(evidence.audioEvents.filter(event => event.event === "play").every(event => event.offsetSeconds === 0)).toBe(true);
  expect(evidence.audioEvents.some(event => event.event === "stop" && event.role === "voice" && event.key === "voice/tomas-road")).toBe(true);
});

test("audio decode failure stays muted and does not block dialogue progression", async ({ page }) => {
  await openPlayer(page);
  await page.evaluate(() => { if (window.__vnTestHooks) window.__vnTestHooks.controls.corruptAudioKey = "voice/tomas-road"; });
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  await expect(page.getByText("Sound is unavailable. You can continue without it.", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue dialogue" })).toBeEnabled();
  expect((await metrics(page)).audioEvents.some(event => event.event === "decode-error" && event.key === "voice/tomas-road")).toBe(true);
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("It ends at the old hall.", { exact: true })).toBeVisible();
  expect((await currentView(page)).view.depth).toBe(2);
});

test("delayed visual failure retains the old complete screen and resource retry does not resubmit choice", async ({ page }) => {
  await openPlayer(page);
  await unlockSound(page);
  await toChoice(page);
  await waitForCandidatePreparation(page);
  await clearPreparedCaches(page);
  const choiceView = await currentView(page);
  const beforeCalls = await metrics(page);
  await page.evaluate(() => {
    if (window.__vnTestHooks) window.__vnTestHooks.controls.failNext = { key: "background/old-hall", delayMs: 450 };
  });
  await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).click();
  await expect(page.locator(".loading-card")).toContainText("Loading next scene…");
  await expect(page.getByRole("button", { name: "Take the narrow passage.", exact: true })).toBeDisabled();
  await expect(page.locator("[data-background='background/river-night']")).toBeVisible();
  await expect(page.locator("[data-layer='mira']")).toBeVisible();
  await expect(page.locator("[data-layer='tomas']")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Scene unavailable", exact: true })).toBeVisible();
  await expect(page.getByText("The next scene could not load.", { exact: true })).toBeVisible();
  expect(await page.locator(".state-layer.is-failure").evaluate(element => getComputedStyle(element).backdropFilter)).toBe("none");
  const retryBounds = await page.getByRole("button", { name: "Retry", exact: true }).boundingBox();
  expect(retryBounds?.width).toBeGreaterThanOrEqual(220);
  expect(retryBounds?.height).toBeGreaterThanOrEqual(56);

  const committed = await currentView(page);
  const saveAfterCommit = await save(page);
  expect(committed.view.cursor).not.toBe(choiceView.view.cursor);
  expect(committed.view.depth).toBe(choiceView.view.depth + 1);
  let evidence = await metrics(page);
  expect(evidence.readerCalls.advance).toBe(beforeCalls.readerCalls.advance);
  expect(evidence.readerCalls.choose).toBe(beforeCalls.readerCalls.choose + 1);
  expect(evidence.mediaRequests.filter(item => item.key === "background/old-hall").map(item => item.status)).toEqual(["available", "unavailable"]);

  await capture(page, "desktop-retry.png");
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
  await expect(page.locator("[data-background='background/old-hall']")).toBeVisible();
  const afterRetry = await currentView(page);
  expect(afterRetry.view.cursor).toBe(committed.view.cursor);
  expect(afterRetry.view.depth).toBe(committed.view.depth);
  expect(await save(page)).toBe(saveAfterCommit);
  evidence = await metrics(page);
  expect(evidence.readerCalls.advance).toBe(beforeCalls.readerCalls.advance);
  expect(evidence.readerCalls.choose).toBe(beforeCalls.readerCalls.choose + 1);
  expect(evidence.retries.resource).toBe(1);
  expect(evidence.mediaRequests.filter(item => item.key === "background/old-hall")).toHaveLength(3);
  expect(evidence.mediaRequests.filter(item => item.key === "background/old-hall").slice(-2).map(item => item.status)).toEqual(["unavailable", "available"]);
  expect(evidence.releases).toBeGreaterThan(0);
});

test("a late resource from a previous host context is discarded and released", async ({ page }) => {
  await openPlayer(page);
  await toChoice(page);
  await waitForCandidatePreparation(page);
  await clearPreparedCaches(page);
  const choice = await currentView(page);
  await page.evaluate(() => {
    if (window.__vnTestHooks) window.__vnTestHooks.controls.delayLateNext = { key: "background/old-hall", delayMs: 650 };
  });
  await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).click();
  await expect(page.locator(".loading-card")).toContainText("Loading next scene…");
  const committed = await currentView(page);
  expect(committed.view.cursor).not.toBe(choice.view.cursor);
  expect(committed.view.depth).toBe(choice.view.depth + 1);
  await page.evaluate(async () => { await window.__vnTestHooks?.switchContext("reader-two"); });
  await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
  await expect.poll(async () => (await metrics(page)).lateDiscards).toBeGreaterThan(0);
  const view = await currentView(page);
  expect(view.view.cursor).toBe(committed.view.cursor);
  expect(view.view.depth).toBe(committed.view.depth);
  expect(view.view.interaction.kind).toBe("advance");
  expect(view.view.scene.background?.key).toBe("background/old-hall");
  const evidence = await metrics(page);
  expect(evidence.lateDiscarded.some(row => row.key === "background/old-hall"
    && row.contextScope === "demo-public" && row.commit === committed.view.cursor)).toBe(true);
  expect(evidence.mediaRequests.some(row => row.key === "background/old-hall" && row.contextScope === "reader-two" && row.status === "available")).toBe(true);
  expect(evidence.aborts).toBeGreaterThan(0);
  expect(evidence.releases).toBeGreaterThan(0);
});

test("unknown storage acknowledgement retries saved bytes without advancing twice", async ({ page }) => {
  await openPlayer(page);
  const before = await currentView(page);
  await page.evaluate(() => { if (window.__vnTestHooks) window.__vnTestHooks.controls.failNextSave = true; });
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByRole("heading", { name: "Save status unknown", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Retry save", exact: true })).toBeEnabled();
  const calls = (await metrics(page)).readerCalls.advance;
  expect(calls).toBe(1);
  const pendingWrite = (await metrics(page)).saveWrites.at(-1);
  if (!pendingWrite) throw new Error("The unknown save request bytes were not recorded.");
  await page.evaluate(async () => { await window.__vnTestHooks?.clearPreparedCaches(); });
  await expect(page.getByRole("heading", { name: "Save status unknown", exact: true })).toBeVisible();
  expect((await metrics(page)).saveWrites.at(-1)).toEqual(pendingWrite);
  await page.getByRole("button", { name: "Retry save", exact: true }).click();
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  const after = await currentView(page);
  expect(after.view.cursor).not.toBe(before.view.cursor);
  expect((await metrics(page)).readerCalls.advance).toBe(1);
  const writes = (await metrics(page)).saveWrites;
  expect(writes.at(-1)).toEqual(writes.at(-2));
});
