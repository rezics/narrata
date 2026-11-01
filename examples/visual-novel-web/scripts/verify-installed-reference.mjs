import { createHash } from "node:crypto";
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { copyFile, lstat, mkdir, readFile, readdir, realpath, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";

const EXPECTED = Object.freeze({
  packageVersion: "0.1.0-alpha.1",
  sourceCommit: "2745b876a66d079ff8ecd573239c27e8fb3a1d3e",
  tarballBytes: 2_275_288,
  tarballSha256: "2fae09fba6fffcbfefb395c524a030ed6d0c320634035586e99167574307344e",
});
const RESULT_MARKER = "NARRATA_INSTALLED_PUBLICATION=";
const TRACKED_ROOTS = ["examples/visual-novel-web", "products/visual-novel-demo"];
const PUBLICATION_FILES = ["examples/visual-novel-web/src/generated/publication.ts"];
const here = dirname(fileURLToPath(import.meta.url));
const sourceRoot = resolve(here, "../../../");

const args = parseArguments(process.argv.slice(2));
if (args.publishOnly) {
  await publishUsingInstalledApi();
} else {
  await verifyInstalledReference(args);
}

function parseArguments(values) {
  const result = { publishOnly: false };
  for (let index = 0; index < values.length; index++) {
    const value = values[index];
    if (value === "--publish-only") result.publishOnly = true;
    else if (value === "--tarball" || value === "--receipt-label") {
      const next = values[++index];
      if (!next || next.startsWith("--")) throw new Error(`${value} requires an explicit value.`);
      if (value === "--tarball") result.tarball = next;
      else result.receiptLabel = next;
    } else throw new Error(`Unknown argument: ${value}`);
  }
  if (!result.publishOnly) {
    if (!result.tarball) throw new Error("Pass the fixed SDK tarball with --tarball.");
    if (!result.receiptLabel || !/^[A-Za-z0-9][A-Za-z0-9._-]{2,63}$/.test(result.receiptLabel)) {
      throw new Error("Pass a fresh receipt label (3–64 letters, digits, dots, underscores or hyphens) with --receipt-label.");
    }
  }
  return result;
}

async function publishUsingInstalledApi() {
  const [{ loadVnAuthoring }, { publishReferenceVn }] = await Promise.all([
    import("@rezics/narrata/vn/server"),
    import("../../../products/visual-novel-demo/scripts/publish-lib.mjs"),
  ]);
  const root = resolve(here, "../../../");
  const product = resolve(root, "products/visual-novel-demo");
  const result = await publishReferenceVn({
    authoring: await loadVnAuthoring(),
    sourceDirectory: product,
    packPath: join(product, "story.narpack"),
    publicStoryDirectory: join(product, "public/story"),
    generatedPublicationPath: join(root, "examples/visual-novel-web/src/generated/publication.ts"),
  });
  const summary = {
    artifactId: result.artifact_id,
    lock: result.lock,
    objects: result.objects,
    manifest: result.manifest,
    packBytes: result.pack.byteLength,
    packSha256: hash("sha256", result.pack),
    packSha512: hash("sha512", result.pack),
  };
  console.log(`${RESULT_MARKER}${JSON.stringify(summary)}`);
}

async function verifyInstalledReference({ tarball, receiptLabel }) {
  const receiptRoot = join(tmpdir(), ".temp", "narrata-vn-reference", receiptLabel);
  const consumerRoot = join(receiptRoot, "consumer");
  const receiptPath = join(receiptRoot, "receipt.json");
  const tempRoot = join(tmpdir(), ".temp", "narrata-vn-reference");
  await mkdir(tempRoot, { recursive: true });
  await mkdir(receiptRoot, { recursive: false });

  const receipt = {
    label: receiptLabel,
    status: "running",
    startedAt: new Date().toISOString(),
    browserPlugin: "Browser plugin not available in this session; repository Playwright Chromium was used.",
    fixedSdk: { ...EXPECTED },
    inputTarball: {},
    source: {},
    runtimeVersions: {},
    consumer: {},
    publicationInventories: [],
    publicationRuns: {},
    commands: [],
    evidence: {},
    failures: [],
  };
  const sanitize = value => sanitizeText(String(value), [sourceRoot, consumerRoot, receiptRoot]);
  const save = async () => writeFile(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, "utf8");

  try {
    const sourceBefore = await sourceIdentity();
    if (sourceBefore.relevantWorkingTree.length) throw new Error("Commit the relevant reference host and verification inputs before running the independent consumer.");
    receipt.source = sourceBefore;
    await save();
    const inputPath = resolve(sourceRoot, tarball);
    const inputBytes = await readFile(inputPath);
    const tarballIdentity = {
      path: sanitize(inputPath),
      bytes: inputBytes.byteLength,
      sha256: hash("sha256", inputBytes),
      sha512: `sha512-${createHash("sha512").update(inputBytes).digest("base64")}`,
    };
    receipt.inputTarball = tarballIdentity;
    if (tarballIdentity.bytes !== EXPECTED.tarballBytes || tarballIdentity.sha256 !== EXPECTED.tarballSha256) {
      throw new Error("The explicit SDK tarball does not match the fixed VN preparation input.");
    }

    const trackedFiles = listTrackedReferenceFiles();
    receipt.source.inputFiles = await hashInputs(trackedFiles);
    await mkdir(consumerRoot, { recursive: false });
    await copyTrackedFiles(trackedFiles, consumerRoot);
    const sdkDirectory = join(consumerRoot, "sdk");
    await mkdir(sdkDirectory, { recursive: true });
    const copiedTarball = join(sdkDirectory, "vn-preparation.tgz");
    await copyFile(inputPath, copiedTarball);

    const exampleRoot = join(consumerRoot, "examples/visual-novel-web");
    const workspaceResolutionGuard = await removeWorkspaceSdkResolutionAllowance(exampleRoot);
    const packagePath = join(exampleRoot, "package.json");
    const packageJson = JSON.parse(await readFile(packagePath, "utf8"));
    packageJson.dependencies ??= {};
    packageJson.dependencies["@rezics/narrata"] = "file:../../sdk/vn-preparation.tgz";
    await writeFile(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`, "utf8");

    const environment = { ...process.env, CI: "1" };
    const npm = process.platform === "win32" ? "npm.cmd" : "npm";
    const npx = process.platform === "win32" ? "npx.cmd" : "npx";
    const nodePublisher = join(exampleRoot, "scripts/verify-installed-reference.mjs");
    const publicationRoot = join(consumerRoot, "products/visual-novel-demo");
    const inventoryBefore = await publicationInventory(consumerRoot);
    receipt.publicationInventories.push({ phase: "before-publication", ...inventoryBefore });

    await runRecorded("npm-lock", npm, ["install", "--package-lock-only", "--ignore-scripts", "--no-audit", "--no-fund"], exampleRoot, receipt, save, sanitize, environment);
    await runRecorded("npm-ci", npm, ["ci", "--no-audit", "--no-fund"], exampleRoot, receipt, save, sanitize, environment);

    const installedSdk = await checkInstalledPackage(consumerRoot, exampleRoot, inputBytes);
    receipt.consumer = {
      root: sanitize(await realpath(consumerRoot)),
      outsideCheckout: true,
      sdk: installedSdk,
      sdkSourceCommit: EXPECTED.sourceCommit,
      workspaceResolutionAllowances: workspaceResolutionGuard,
    };

    receipt.runtimeVersions.node = await runVersion("node", process.execPath, ["--version"], exampleRoot, receipt, save, sanitize, environment);
    receipt.runtimeVersions.npm = await runVersion("npm", npm, ["--version"], exampleRoot, receipt, save, sanitize, environment);
    receipt.runtimeVersions.bun = await runVersion("bun", "bun", ["--version"], exampleRoot, receipt, save, sanitize, environment);
    receipt.runtimeVersions.playwright = await runVersion("playwright", npx, ["playwright", "--version"], exampleRoot, receipt, save, sanitize, environment);

    const nodeResult = await runPublisher("node", process.execPath, nodePublisher, exampleRoot, receipt, save, sanitize, environment);
    receipt.publicationRuns.node = nodeResult;
    const afterNode = await publicationInventory(consumerRoot);
    receipt.publicationInventories.push({ phase: "after-node-publication", ...afterNode });
    assertInventoryEqual(inventoryBefore, afterNode, "Node publication");
    await assertPackMatches(publicationRoot, nodeResult);

    const bunResult = await runPublisher("bun", "bun", nodePublisher, exampleRoot, receipt, save, sanitize, environment);
    receipt.publicationRuns.bun = bunResult;
    assert.deepEqual(bunResult, nodeResult, "Node and Bun public server builds did not produce the same artifact, lock, objects and pack.");
    const afterBun = await publicationInventory(consumerRoot);
    receipt.publicationInventories.push({ phase: "after-bun-publication", ...afterBun });
    assertInventoryEqual(afterNode, afterBun, "Bun publication");
    await assertPackMatches(publicationRoot, bunResult);

    await runRecorded("build-host", npm, ["run", "build"], exampleRoot, receipt, save, sanitize, environment);
    await runRecorded("install-chromium", npx, ["playwright", "install", "chromium"], exampleRoot, receipt, save, sanitize, environment);
    const browserRun = await runRecorded("installed-browser-journeys", npm, ["run", "test:e2e"], exampleRoot, receipt, save, sanitize, environment);
    receipt.evidence.browserTests = parseBrowserTestSummary(`${browserRun.stdout}\n${browserRun.stderr}`);

    const afterBrowser = await publicationInventory(consumerRoot);
    receipt.publicationInventories.push({ phase: "after-browser-previews", ...afterBrowser });
    assertInventoryEqual(afterBun, afterBrowser, "Browser previews");
    receipt.evidence = { ...receipt.evidence, ...(await summarizeBrowserEvidence(consumerRoot)) };
    receipt.sourceAfter = await sourceIdentity();
    receipt.sourceAfter.inputFiles = await hashInputs(listTrackedReferenceFiles());
    assert.deepEqual(receipt.sourceAfter.inputFiles, receipt.source.inputFiles, "Relevant workspace source changed during the independent consumer run.");
    if (receipt.sourceAfter.commit !== receipt.source.commit || receipt.sourceAfter.relevantWorkingTree.length) {
      throw new Error("Relevant source was not committed or changed during the independent consumer run.");
    }
    receipt.status = "passed";
  } catch (error) {
    receipt.status = "failed";
    receipt.failures.push(sanitize(error instanceof Error ? error.message : error));
    try {
      receipt.sourceAfter = await sourceIdentity();
      receipt.sourceAfter.inputFiles = await hashInputs(listTrackedReferenceFiles());
    } catch (sourceError) {
      receipt.failures.push(sanitize(`Could not verify source after the run: ${sourceError instanceof Error ? sourceError.message : sourceError}`));
    }
  } finally {
    receipt.finishedAt = new Date().toISOString();
    await save();
  }

  console.log(JSON.stringify({ status: receipt.status, receipt: sanitize(receiptPath), consumer: sanitize(consumerRoot), failures: receipt.failures }, null, 2));
  if (receipt.status !== "passed") process.exitCode = 1;
}

async function runPublisher(id, command, script, cwd, receipt, save, sanitize, environment) {
  const result = await runRecorded(id, command, [script, "--publish-only"], cwd, receipt, save, sanitize, environment);
  const line = result.stdout.split(/\r?\n/).find(value => value.startsWith(RESULT_MARKER));
  if (!line) throw new Error(`${id} did not return the public publication summary.`);
  const summary = JSON.parse(line.slice(RESULT_MARKER.length));
  const pack = await readFile(join(resolve(cwd, "../../products/visual-novel-demo"), "story.narpack"));
  if (summary.packBytes !== pack.byteLength || summary.packSha256 !== hash("sha256", pack) || summary.packSha512 !== hash("sha512", pack)) {
    throw new Error(`${id} summary did not match the written story.narpack bytes.`);
  }
  return summary;
}

async function runRecorded(id, command, commandArgs, cwd, receipt, save, sanitize, environment) {
  console.log(`[installed-vn] ${id}: ${sanitize(command)} ${commandArgs.map(sanitize).join(" ")}`);
  const started = Date.now();
  const record = {
    id,
    command: sanitize(command),
    args: commandArgs.map(sanitize),
    cwd: sanitize(cwd),
    exitCode: null,
    signal: null,
    elapsedMs: 0,
    stdout: "",
    stderr: "",
  };
  receipt.commands.push(record);
  await save();
  let result;
  try {
    result = await new Promise((resolveResult, rejectResult) => {
      const child = spawn(command, commandArgs, {
        cwd,
        env: environment,
        windowsHide: true,
        shell: process.platform === "win32" && (command.endsWith(".cmd") || command.endsWith(".bat")),
        stdio: ["ignore", "pipe", "pipe"],
      });
      let stdout = "";
      let stderr = "";
      child.stdout.setEncoding("utf8");
      child.stderr.setEncoding("utf8");
      child.stdout.on("data", chunk => { stdout = keepTail(stdout, chunk); });
      child.stderr.on("data", chunk => { stderr = keepTail(stderr, chunk); });
      child.once("error", rejectResult);
      child.once("close", (code, signal) => resolveResult({ code, signal, stdout, stderr }));
    });
  } catch (error) {
    const failure = `${id} could not start: ${error instanceof Error ? error.message : error}`;
    record.elapsedMs = Date.now() - started;
    record.launchFailure = sanitize(failure);
    receipt.failures.push(sanitize(failure));
    await save();
    throw new Error(failure);
  }
  record.exitCode = result.code;
  record.signal = result.signal;
  record.elapsedMs = Date.now() - started;
  record.stdout = sanitize(result.stdout);
  record.stderr = sanitize(result.stderr);
  await save();
  if (result.code !== 0) {
    const failure = `${id} failed with exit code ${result.code ?? "unknown"}${result.stderr ? `: ${sanitize(result.stderr.slice(-1600))}` : ""}`;
    receipt.failures.push(failure);
    await save();
    throw new Error(failure);
  }
  return result;
}

async function runVersion(id, command, args, cwd, receipt, save, sanitize, environment) {
  const result = await runRecorded(`version-${id}`, command, args, cwd, receipt, save, sanitize, environment);
  return result.stdout.trim().split(/\r?\n/).at(-1) ?? "unknown";
}

async function checkInstalledPackage(consumerRoot, exampleRoot, tarballBytes) {
  const consumerReal = await realpath(consumerRoot);
  const sourceReal = await realpath(sourceRoot);
  if (isWithin(sourceReal, consumerReal) || isWithin(consumerReal, sourceReal)) {
    throw new Error("The independent consumer realpath overlaps the source checkout.");
  }
  const sdkPath = join(exampleRoot, "node_modules/@rezics/narrata");
  const sdkStat = await lstat(sdkPath);
  if (!sdkStat.isDirectory() || sdkStat.isSymbolicLink()) throw new Error("The independent SDK install is not a regular directory.");
  const sdkReal = await realpath(sdkPath);
  assertWithin(consumerReal, sdkReal, "Installed SDK");
  if (isWithin(sourceReal, sdkReal)) throw new Error("The installed SDK resolved back into the source checkout.");

  const lock = JSON.parse(await readFile(join(exampleRoot, "package-lock.json"), "utf8"));
  const locked = lock.packages?.["node_modules/@rezics/narrata"];
  const expectedIntegrity = `sha512-${createHash("sha512").update(tarballBytes).digest("base64")}`;
  if (!locked || locked.version !== EXPECTED.packageVersion || locked.integrity !== expectedIntegrity
    || locked.resolved !== "file:../../sdk/vn-preparation.tgz") {
    throw new Error("The installed SDK lock entry does not match the fixed local tarball identity.");
  }
  const installedPackage = JSON.parse(await readFile(join(sdkReal, "package.json"), "utf8"));
  if (installedPackage.version !== EXPECTED.packageVersion) throw new Error("The installed SDK version differs from the fixed package version.");
  const serverExport = installedPackage.exports?.["./vn/server"]?.import;
  if (typeof serverExport !== "string") throw new Error("The installed SDK does not expose its public VN server entry.");
  const serverEntry = await realpath(resolve(sdkReal, serverExport));
  assertWithin(sdkReal, serverEntry, "Public VN server entry");

  return {
    realpath: sdkReal,
    insideConsumer: true,
    notWorkspaceSymlink: true,
    version: installedPackage.version,
    packageJsonPath: join(sdkReal, "package.json"),
    serverEntry,
    lock: { resolved: locked.resolved, version: locked.version, integrity: locked.integrity },
  };
}

async function sourceIdentity() {
  const commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: sourceRoot, encoding: "utf8" }).trim();
  const status = execFileSync("git", ["status", "--porcelain", "--", ...TRACKED_ROOTS, "Taskfile.yml", "docs/development/toolchain.md", "docs/integrations/rezics.md"], { cwd: sourceRoot, encoding: "utf8" });
  return { commit, relevantWorkingTree: status.split(/\r?\n/).filter(Boolean), checkedPaths: [...TRACKED_ROOTS, "Taskfile.yml", "docs/development/toolchain.md", "docs/integrations/rezics.md"] };
}

function listTrackedReferenceFiles() {
  const listing = execFileSync("git", ["ls-files", "-z", "--", ...TRACKED_ROOTS], { cwd: sourceRoot });
  return listing.toString("utf8").split("\0").filter(Boolean).sort();
}

async function hashInputs(paths) {
  const result = [];
  for (const path of paths) {
    const bytes = await readFile(resolve(sourceRoot, path));
    result.push({ path, bytes: bytes.byteLength, sha256: hash("sha256", bytes) });
  }
  return result;
}

async function copyTrackedFiles(paths, consumerRoot) {
  for (const path of paths) {
    const source = resolve(sourceRoot, path);
    const target = resolve(consumerRoot, path);
    await mkdir(dirname(target), { recursive: true });
    await copyFile(source, target);
  }
}

async function removeWorkspaceSdkResolutionAllowance(exampleRoot) {
  const configPath = join(exampleRoot, "vite.config.ts");
  const source = await readFile(configPath, "utf8");
  const installedConsumerConfig = source
    .replace(/^const packageRoot = resolve\(here, "\.\.\/\.\.\/packages\/narrata\/web"\);\r?\n/m, "")
    .replace("fs: { allow: [here, product, packageRoot] }", "fs: { allow: [here, product] }");
  if (installedConsumerConfig === source || /packageRoot|packages\/narrata\/web/.test(installedConsumerConfig)) {
    throw new Error("Could not remove the copied Vite workspace SDK resolution allowance.");
  }
  await writeFile(configPath, installedConsumerConfig, "utf8");
  const verifiedConfig = await readFile(configPath, "utf8");
  if (/packageRoot|packages\/narrata\/web/.test(verifiedConfig) || !verifiedConfig.includes("fs: { allow: [here, product] }")) {
    throw new Error("The copied Vite config still permits workspace SDK resolution.");
  }
  return { verified: true, allowlist: ["here", "product"], workspaceSdkPathAbsent: true };
}

function parseBrowserTestSummary(output) {
  const totalMatch = /Running\s+(\d+)\s+tests?\s+using/.exec(output);
  const passedMatch = /(\d+)\s+passed(?:\s+\([^)]+\))?/.exec(output);
  if (!totalMatch || !passedMatch) throw new Error("The installed Playwright run did not report a structured test count.");
  const total = Number(totalMatch[1]);
  const passed = Number(passedMatch[1]);
  if (!Number.isSafeInteger(total) || !Number.isSafeInteger(passed) || total < 1 || passed !== total) {
    throw new Error(`The installed Playwright run reported ${passed}/${total} passing tests.`);
  }
  return { total, passed };
}

async function publicationInventory(root) {
  const paths = [];
  const productRoot = join(root, "products/visual-novel-demo");
  await walkFiles(root, productRoot, paths);
  for (const path of PUBLICATION_FILES) paths.push(join(root, path));
  paths.sort();
  const files = [];
  for (const path of paths) {
    const bytes = await readFile(path);
    files.push({ path: relative(root, path).split(sep).join("/"), bytes: bytes.byteLength, sha256: hash("sha256", bytes) });
  }
  return { fileCount: files.length, inventorySha256: hash("sha256", Buffer.from(JSON.stringify(files))), files };
}

async function walkFiles(root, directory, output) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) await walkFiles(root, path, output);
    else if (entry.isFile()) output.push(path);
  }
}

function assertInventoryEqual(before, after, phase) {
  if (before.inventorySha256 !== after.inventorySha256 || before.fileCount !== after.fileCount) {
    throw new Error(`${phase} changed the complete published file inventory or one of its hashes.`);
  }
}

async function assertPackMatches(productRoot, summary) {
  const packPath = join(productRoot, "story.narpack");
  const bytes = await readFile(packPath);
  if (summary.packBytes !== bytes.byteLength || summary.packSha256 !== hash("sha256", bytes)) {
    throw new Error("The publication summary does not match the complete story.narpack file.");
  }
}

async function summarizeBrowserEvidence(consumerRoot) {
  const root = join(consumerRoot, ".temp/goal/runs");
  const screenshots = [];
  const measurements = [];
  await collectEvidence(root, screenshots, measurements);
  const parsed = [];
  for (const path of measurements) {
    try { parsed.push(JSON.parse(await readFile(path, "utf8"))); } catch { /* Failed runs still preserve malformed evidence for review. */ }
  }
  const candidateDecode = parsed.flatMap(item => item.mediaDecodes ?? []).some(item => item.phase === "candidate");
  const actualDecode = parsed.flatMap(item => item.mediaDecodes ?? []).some(item => item.kind === "audio")
    && parsed.flatMap(item => item.mediaDecodes ?? []).some(item => item.kind === "image");
  const releaseObserved = parsed.some(item => item.releases > 0);
  if (screenshots.length === 0 || !candidateDecode || !actualDecode || !releaseObserved) {
    throw new Error("The installed Chromium evidence is missing screenshots, prepared image/audio decode, or released resource leases.");
  }
  const distributionPath = join(root, "vn-player/distribution.json");
  const distribution = JSON.parse(await readFile(distributionPath, "utf8"));
  const authoringPath = join(root, "vn-authoring/publication-b-metrics.json");
  const authoring = JSON.parse(await readFile(authoringPath, "utf8"));
  const candidateBudgetPath = join(root, "vn-authoring/candidate-budget-metrics.json");
  const candidateBudget = JSON.parse(await readFile(candidateBudgetPath, "utf8"));
  const candidateAudioPath = join(root, "vn-authoring/candidate-audio-metrics.json");
  const candidateAudio = JSON.parse(await readFile(candidateAudioPath, "utf8"));
  const distributionAudioPlayCount = distribution.media.audioPlayCountAfterPreparation - distribution.media.audioPlayCountBeforePreparation;
  const candidateAudioPlayCount = candidateAudio.playCountAfterPreparation - candidateAudio.playCountBeforePreparation;
  if (distribution.wasm.firstScreenReaderModuleCount !== 1 || distribution.wasm.firstScreenAuthoringModuleRequested
    || distributionAudioPlayCount !== 0) {
    throw new Error("The installed first-screen or candidate audio evidence does not prove lazy reader loading and silent preparation.");
  }
  if (candidateAudio.atChoice !== true || !["complete", "partial"].includes(candidateAudio.candidateStatus)
    || candidateAudio.futureVoiceRequest?.status !== "available" || candidateAudio.futureVoiceRequest.networkBytes < 1
    || candidateAudio.futureVoiceDecode?.phase !== "candidate" || candidateAudio.futureVoiceDecode?.kind !== "audio"
    || candidateAudio.futureVoiceDecode?.bytes !== candidateAudio.futureVoiceRequest.bytes
    || candidateAudioPlayCount !== 0 || candidateAudio.futureVoicePlayCount !== 0) {
    throw new Error("The installed candidate did not decode the future voice without playing it.");
  }
  if (candidateBudget.playableChoice?.playerStatus !== "ready" || !candidateBudget.playableChoice.cursorAdvanced
    || candidateBudget.playableChoice.choiceCalls !== 1) {
    throw new Error("The installed bounded candidate did not prove a playable branch after partial preparation.");
  }
  if (candidateBudget.candidateStatus?.status !== "partial"
    || candidateBudget.candidateBounds?.maxCandidateRequests !== 1
    || candidateBudget.attempted?.length !== 1 || candidateBudget.attempted[0]?.revision !== "1"
    || JSON.stringify(candidateBudget.refusedVersions) !== JSON.stringify(["2"])
    || candidateBudget.refusedByCandidateRequestBound?.maxCandidateRequests !== 1
    || candidateBudget.refusedByCandidateRequestBound?.revision !== "2") {
    throw new Error("The installed budget evidence did not tie the refused revision to the one-request limit.");
  }
  const logicalRequests = parsed.flatMap(item => item.logicalRequests ?? []);
  const mediaRequests = parsed.flatMap(item => item.mediaRequests ?? []);
  const retries = parsed.reduce((total, item) => ({
    resource: total.resource + (item.retries?.resource ?? 0),
    audio: total.audio + (item.retries?.audio ?? 0),
    logical: total.logical + (item.retries?.logical ?? 0),
    save: total.save + (item.retries?.save ?? 0),
  }), { resource: 0, audio: 0, logical: 0, save: 0 });
  return {
    screenshotCount: screenshots.length,
    measurementCount: measurements.length,
    screenshots: screenshots.map(path => relative(consumerRoot, path).split(sep).join("/")),
    measurements: measurements.map(path => relative(consumerRoot, path).split(sep).join("/")),
    candidateMediaDecodeObserved: candidateDecode,
    actualImageAndAudioDecodeObserved: actualDecode,
    firstScreenReaderModuleCount: distribution.wasm.firstScreenReaderModuleCount,
    firstScreenAuthoringModuleRequested: distribution.wasm.firstScreenAuthoringModuleRequested,
    candidateAudioDecodeCount: candidateAudio.futureVoiceDecode ? 1 : 0,
    candidateAudioPlayCount: candidateAudioPlayCount,
    candidateAudio: {
      artifactId: candidateAudio.artifactId,
      status: candidateAudio.candidateStatus,
      bounds: candidateAudio.bounds,
      futureVoiceRequest: candidateAudio.futureVoiceRequest,
      futureVoiceDecode: candidateAudio.futureVoiceDecode,
      playCountBeforePreparation: candidateAudio.playCountBeforePreparation,
      playCountAfterPreparation: candidateAudio.playCountAfterPreparation,
      futureVoicePlayCount: candidateAudio.futureVoicePlayCount,
    },
    releasedResourceLeaseObserved: releaseObserved,
    logicalRequests,
    mediaRequests,
    retries,
    releases: parsed.reduce((total, item) => total + (item.releases ?? 0), 0),
    distribution,
    publicationB: {
      artifactId: authoring.artifactId,
      firstText: authoring.firstText,
      candidateStatus: authoring.candidateStatus?.status,
      logicalRequests: authoring.metrics.logicalRequests,
      mediaRequests: authoring.metrics.mediaRequests,
      mediaDecodes: authoring.metrics.mediaDecodes,
      audioPlayCount: authoring.metrics.audioEvents.filter(item => item.event === "play").length,
      releases: authoring.metrics.releases,
      aborts: authoring.metrics.aborts,
    },
    candidateBudget: {
      artifactId: candidateBudget.artifactId,
      bounds: candidateBudget.candidateBounds,
      playerStatus: candidateBudget.playerStatus,
      atChoice: candidateBudget.atChoice,
      status: candidateBudget.candidateStatus?.status,
      completeProjection: candidateBudget.candidateStatus?.completeProjection,
      mediaPrepared: candidateBudget.candidateStatus?.mediaPrepared,
      mediaUnavailable: candidateBudget.candidateStatus?.mediaUnavailable,
      attempted: candidateBudget.attempted,
      refusedVersions: candidateBudget.refusedVersions,
      refusedByCandidateRequestBound: candidateBudget.refusedByCandidateRequestBound,
      playableChoice: candidateBudget.playableChoice,
    },
  };
}

async function collectEvidence(directory, screenshots, measurements) {
  let entries;
  try { entries = await readdir(directory, { withFileTypes: true }); } catch { return; }
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) await collectEvidence(path, screenshots, measurements);
    else if (entry.isFile()) {
      if (/\.(png|webp)$/i.test(entry.name)) screenshots.push(path);
      else if (/\.json$/i.test(entry.name)) measurements.push(path);
    }
  }
}

function assertWithin(root, target, label) {
  if (!isWithin(root, target)) throw new Error(`${label} resolved outside its required package directory.`);
}

function isWithin(root, target) {
  const path = relative(root, target);
  return path === "" || (!path.startsWith(`..${sep}`) && path !== ".." && !isAbsolute(path));
}

function hash(algorithm, bytes) {
  return createHash(algorithm).update(bytes).digest("hex");
}

function keepTail(current, next) {
  return `${current}${next}`.slice(-16_000);
}

function sanitizeText(value, roots) {
  let result = value;
  for (const root of roots) {
    result = result.split(root).join(root === sourceRoot ? "<workspace>" : root.endsWith("consumer") ? "<consumer>" : "<receipt>");
    result = result.split(root.replaceAll("\\", "/")).join(root === sourceRoot ? "<workspace>" : root.endsWith("consumer") ? "<consumer>" : "<receipt>");
  }
  return result;
}
