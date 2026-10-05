import { describe, expect, it } from "vitest";

import { CborError, compareBytes, decode, encode } from "../src/cbor";

const hex = (value: string) => Uint8Array.from(value.match(/../g) ?? [], (byte) => Number.parseInt(byte, 16));

describe("canonical CBOR", () => {
  it("encodes integers in their shortest form", () => {
    const cases: [number, string][] = [
      [0, "00"],
      [23, "17"],
      [24, "1818"],
      [255, "18ff"],
      [256, "190100"],
      [65_535, "19ffff"],
      [65_536, "1a00010000"],
      [0xffff_ffff, "1affffffff"],
      [0x1_0000_0000, "1b0000000100000000"],
      [Number.MAX_SAFE_INTEGER, "1b001fffffffffffff"],
    ];
    for (const [value, encoded] of cases) {
      expect(encode(value)).toEqual(hex(encoded));
      expect(decode(hex(encoded))).toBe(value);
    }
  });

  it("round-trips byte strings, text, arrays and null", () => {
    const value = [new Uint8Array([1, 2, 3]), "narrata", [null, 7, []], new Uint8Array(300)];
    expect(decode(encode(value))).toEqual(value);
    expect(encode(["a", null])).toEqual(hex("826161f6"));
  });

  it("refuses everything outside the profile", () => {
    const refused = [
      "1817", // non-minimal integer
      "190017",
      "1b00000000ffffffff",
      "1b0020000000000000", // beyond 2^53 - 1
      "20", // negative integer
      "a0", // map
      "c0", // tag
      "f5", // true
      "f93c00", // half float
      "9f00ff", // indefinite array
      "5f", // indefinite byte string
      "1c", // reserved
      "0000", // trailing bytes
      "8301", // truncated array
      "4401", // truncated byte string
      "62c328", // invalid UTF-8
      "9b0000000100000000", // array longer than the input
    ];
    for (const encoded of refused) expect(() => decode(hex(encoded)), encoded).toThrow(CborError);
    expect(() => encode(-1)).toThrow(CborError);
    expect(() => encode(1.5)).toThrow(CborError);
    expect(() => encode(2 ** 53)).toThrow(CborError);
  });

  it("refuses nesting beyond its depth bound", () => {
    expect(() => decode(new Uint8Array(64).fill(0x81))).toThrow(CborError);
  });

  it("copies byte strings out of the input", () => {
    const input = hex("43010203");
    const decoded = decode(input) as Uint8Array;
    input[1] = 9;
    expect(decoded).toEqual(new Uint8Array([1, 2, 3]));
  });

  it("orders bytes as binary IndexedDB keys", () => {
    const keys = [[1], [], [0, 255], [0], [0, 0]].map((key) => new Uint8Array(key));
    keys.sort(compareBytes);
    expect(keys.map((key) => Array.from(key))).toEqual([[], [0], [0, 0], [0, 255], [1]]);
  });
});
