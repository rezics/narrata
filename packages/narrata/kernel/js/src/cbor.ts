/**
 * The ADR 0003 canonical CBOR profile, limited to what the storage messages use: unsigned
 * integers, byte strings, text strings, arrays and null. Decoding accepts only the shortest,
 * definite-length encoding, so a value has exactly one encoding.
 */

export type Cbor = number | Uint8Array | string | null | readonly Cbor[];

export class CborError extends Error {
  override name = "CborError";
}

const MAX_DEPTH = 16;
const TWO_32 = 0x1_0000_0000;

/** Allocation bounds supplied by a boundary that knows the message's resource budget. */
export interface DecodeLimits {
  maxMessageBytes: number;
  maxStringBytes: number;
  maxArrayItems: number;
  maxNodes: number;
}

class Writer {
  private bytes = new Uint8Array(256);
  private length = 0;

  private reserve(extra: number): void {
    if (this.length + extra <= this.bytes.length) return;
    let size = this.bytes.length * 2;
    while (size < this.length + extra) size *= 2;
    const grown = new Uint8Array(size);
    grown.set(this.bytes.subarray(0, this.length));
    this.bytes = grown;
  }

  byte(value: number): void {
    this.reserve(1);
    this.bytes[this.length++] = value;
  }

  append(value: Uint8Array): void {
    this.reserve(value.length);
    this.bytes.set(value, this.length);
    this.length += value.length;
  }

  head(major: number, value: number): void {
    if (!Number.isSafeInteger(value) || value < 0) {
      throw new CborError(`cannot encode ${value} as an unsigned integer`);
    }
    const prefix = major << 5;
    if (value < 24) {
      this.byte(prefix | value);
    } else if (value <= 0xff) {
      this.byte(prefix | 24);
      this.byte(value);
    } else if (value <= 0xffff) {
      this.byte(prefix | 25);
      this.byte(value >>> 8);
      this.byte(value & 0xff);
    } else if (value < TWO_32) {
      this.byte(prefix | 26);
      for (const shift of [24, 16, 8, 0]) this.byte((value >>> shift) & 0xff);
    } else {
      this.byte(prefix | 27);
      const high = Math.floor(value / TWO_32);
      const low = value % TWO_32;
      for (const part of [high, low]) {
        for (const shift of [24, 16, 8, 0]) this.byte((part >>> shift) & 0xff);
      }
    }
  }

  finish(): Uint8Array {
    return this.bytes.slice(0, this.length);
  }
}

function write(writer: Writer, value: Cbor, depth: number): void {
  if (depth > MAX_DEPTH) throw new CborError("value nests too deeply");
  if (value === null) {
    writer.byte(0xf6);
  } else if (typeof value === "number") {
    writer.head(0, value);
  } else if (typeof value === "string") {
    const bytes = new TextEncoder().encode(value);
    writer.head(3, bytes.length);
    writer.append(bytes);
  } else if (value instanceof Uint8Array) {
    writer.head(2, value.length);
    writer.append(value);
  } else {
    writer.head(4, value.length);
    for (const item of value) write(writer, item, depth + 1);
  }
}

export function encode(value: Cbor): Uint8Array {
  const writer = new Writer();
  write(writer, value, 0);
  return writer.finish();
}

class Reader {
  private offset = 0;
  private nodes = 0;

  constructor(private readonly bytes: Uint8Array, private readonly limits?: DecodeLimits) {}

  private take(length: number): Uint8Array {
    if (length > this.bytes.length - this.offset) throw new CborError("unexpected end of input");
    const value = this.bytes.subarray(this.offset, this.offset + length);
    this.offset += length;
    return value;
  }

  private head(): [number, number] {
    const [initial] = this.take(1);
    const major = initial! >> 5;
    const additional = initial! & 0x1f;
    if (additional < 24) return [major, additional];
    const widths: Record<number, number> = { 24: 1, 25: 2, 26: 4, 27: 8 };
    const width = widths[additional];
    if (width === undefined) {
      throw new CborError(additional === 31 ? "indefinite-length item" : "reserved additional information");
    }
    let value = 0;
    for (const byte of this.take(width)) value = value * 256 + byte;
    const minimum = width === 1 ? 24 : 2 ** (8 * (width / 2));
    if (value < minimum) throw new CborError("non-minimal integer or length");
    if (!Number.isSafeInteger(value)) throw new CborError("integer beyond 2^53 - 1");
    return [major, value];
  }

  item(depth: number): Cbor {
    if (depth > MAX_DEPTH) throw new CborError("value nests too deeply");
    if (this.limits && ++this.nodes > this.limits.maxNodes) throw new CborError("too many CBOR items");
    const [major, value] = this.head();
    if (this.limits && (major === 2 || major === 3) && value > this.limits.maxStringBytes) {
      throw new CborError("CBOR string exceeds allocation limit");
    }
    switch (major) {
      case 0:
        return value;
      case 2:
        return this.take(value).slice();
      case 3:
        try {
          return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(this.take(value));
        } catch {
          throw new CborError("invalid UTF-8 text");
        }
      case 4: {
        if (this.limits && value > this.limits.maxArrayItems) throw new CborError("CBOR array exceeds allocation limit");
        // Every item takes at least one byte, so a longer array cannot be in the input.
        if (value > this.bytes.length - this.offset) throw new CborError("unexpected end of input");
        const items: Cbor[] = [];
        for (let index = 0; index < value; index++) items.push(this.item(depth + 1));
        return items;
      }
      case 7:
        if (value === 22) return null;
        throw new CborError("simple value outside the profile");
      default:
        throw new CborError(`major type ${major} outside the profile`);
    }
  }

  finish(): void {
    if (this.offset !== this.bytes.length) throw new CborError("trailing bytes");
  }
}

export function decode(bytes: Uint8Array, limits?: DecodeLimits): Cbor {
  if (limits && bytes.length > limits.maxMessageBytes) throw new CborError("CBOR message exceeds byte limit");
  const reader = new Reader(bytes, limits);
  const value = reader.item(0);
  reader.finish();
  return value;
}

/** Orders byte strings the way IndexedDB orders binary keys and the contract orders keys. */
export function compareBytes(left: Uint8Array, right: Uint8Array): number {
  const length = Math.min(left.length, right.length);
  for (let index = 0; index < length; index++) {
    const difference = left[index]! - right[index]!;
    if (difference !== 0) return difference;
  }
  return left.length - right.length;
}

export function equalBytes(left: Uint8Array, right: Uint8Array): boolean {
  return compareBytes(left, right) === 0;
}
