import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const sampleRate = 22_050;
const channels = 2;
const bpm = 68;
const beat = 60 / bpm;
const bar = beat * 4;
const chords = [
  ["D minor add 9", [146.83, 220, 293.66, 329.63, 440]],
  ["B flat major 7", [116.54, 233.08, 293.66, 349.23, 440]],
  ["F major 6", [87.31, 174.61, 220, 261.63, 349.23]],
  ["C suspended", [130.81, 196, 261.63, 293.66, 392]],
];
const bars = chords.length * 2;
const durationSeconds = bar * bars;
const frameCount = Math.ceil(sampleRate * durationSeconds);
const samples = new Float64Array(frameCount * channels);
const melody = [0, 2, 1, 3, 2, 4, 3, 1];

function add(frame, value, pan = 0) {
  const left = Math.sqrt((1 - pan) / 2) * value;
  const right = Math.sqrt((1 + pan) / 2) * value;
  samples[frame * channels] += left;
  samples[frame * channels + 1] += right;
}

function envelope(seconds, attack, release, total) {
  const inRamp = Math.min(1, seconds / attack);
  const outRamp = Math.min(1, (total - seconds) / release);
  return Math.max(0, Math.min(inRamp, outRamp));
}

for (let barIndex = 0; barIndex < bars; barIndex++) {
  const [, notes] = chords[barIndex % chords.length];
  const start = barIndex * bar;
  for (let frame = 0; frame < Math.ceil(bar * sampleRate); frame++) {
    const elapsed = frame / sampleRate;
    const absolute = start + elapsed;
    const globalFrame = Math.floor(absolute * sampleRate);
    if (globalFrame >= frameCount) break;
    const fade = envelope(elapsed, 0.72, 0.62, bar);
    let pad = 0;
    for (const [index, frequency] of notes.entries()) {
      const drift = 1 + Math.sin(absolute * 0.16 + index) * 0.0007;
      pad += Math.sin(2 * Math.PI * frequency * drift * absolute) * (index === 0 ? 0.095 : 0.063);
      pad += Math.sin(2 * Math.PI * frequency * 2 * absolute) * 0.009;
    }
    add(globalFrame, pad * fade);
  }

  for (let beatIndex = 0; beatIndex < 4; beatIndex++) {
    const noteIndex = melody[(barIndex * 4 + beatIndex) % melody.length] ?? 0;
    const frequency = notes[noteIndex] ?? notes[0] ?? 220;
    const noteStart = start + beatIndex * beat;
    const noteDuration = beat * 0.82;
    const firstFrame = Math.floor(noteStart * sampleRate);
    const lastFrame = Math.min(frameCount, Math.ceil((noteStart + noteDuration) * sampleRate));
    const pan = beatIndex % 2 === 0 ? -0.18 : 0.18;
    for (let frame = firstFrame; frame < lastFrame; frame++) {
      const elapsed = frame / sampleRate - noteStart;
      const decay = Math.exp(-elapsed * 3.8) * envelope(elapsed, 0.008, 0.05, noteDuration);
      const phase = 2 * Math.PI * frequency * elapsed;
      const pluck = (Math.sin(phase) + Math.sin(phase * 2) * 0.34 + Math.sin(phase * 3) * 0.1) * 0.15 * decay;
      add(frame, pluck, pan);
    }
  }

  for (const beatIndex of [0, 2]) {
    const noteStart = start + beatIndex * beat;
    const frequency = (notes[0] ?? 110) / 2;
    const firstFrame = Math.floor(noteStart * sampleRate);
    const lastFrame = Math.min(frameCount, Math.ceil((noteStart + beat * 0.9) * sampleRate));
    for (let frame = firstFrame; frame < lastFrame; frame++) {
      const elapsed = frame / sampleRate - noteStart;
      const pulse = Math.sin(2 * Math.PI * frequency * elapsed) * Math.exp(-elapsed * 2.5) * 0.11;
      add(frame, pulse, 0);
    }
  }
}

// A short, quiet echo gives the plucked notes a room-like tail without external samples.
const delay = Math.floor(sampleRate * 0.21);
for (let frame = delay; frame < frameCount; frame++) {
  for (let channel = 0; channel < channels; channel++) {
    samples[frame * channels + channel] += samples[(frame - delay) * channels + (1 - channel)] * 0.12;
  }
}

let peak = 0;
for (const sample of samples) peak = Math.max(peak, Math.abs(sample));
const gain = peak > 0 ? 0.82 / peak : 1;
const dataBytes = frameCount * channels * 2;
const wav = Buffer.alloc(44 + dataBytes);
wav.write("RIFF", 0);
wav.writeUInt32LE(36 + dataBytes, 4);
wav.write("WAVE", 8);
wav.write("fmt ", 12);
wav.writeUInt32LE(16, 16);
wav.writeUInt16LE(1, 20);
wav.writeUInt16LE(channels, 22);
wav.writeUInt32LE(sampleRate, 24);
wav.writeUInt32LE(sampleRate * channels * 2, 28);
wav.writeUInt16LE(channels * 2, 32);
wav.writeUInt16LE(16, 34);
wav.write("data", 36);
wav.writeUInt32LE(dataBytes, 40);
for (let index = 0; index < samples.length; index++) {
  const sample = Math.max(-1, Math.min(1, (samples[index] ?? 0) * gain));
  wav.writeInt16LE(Math.round(sample * 32767), 44 + index * 2);
}

const output = resolve(import.meta.dirname, "../public/assets/audio/river-road-theme.wav");
mkdirSync(import.meta.dirname ? resolve(import.meta.dirname, "../public/assets/audio") : resolve("products/visual-novel-demo/public/assets/audio"), { recursive: true });
writeFileSync(output, wav);
console.log(JSON.stringify({ output, sampleRate, channels, durationMs: Math.round(durationSeconds * 1000), bytes: wav.byteLength, chords: chords.map(([name]) => name) }, null, 2));
