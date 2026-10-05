/** Native SHA-256; shared object identity matches the kernel's codec/digest.rs. */
export async function sha256(bytes: Uint8Array): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes)));
}

export async function objectId(kind: number, schema: number, payload: Uint8Array): Promise<Uint8Array> {
  const domain = new TextEncoder().encode("narrata-object\0");
  const bytes = new Uint8Array(domain.length + 4 + payload.length);
  bytes.set(domain);
  const view = new DataView(bytes.buffer);
  view.setUint16(domain.length, kind);
  view.setUint16(domain.length + 2, schema);
  bytes.set(payload, domain.length + 4);
  return sha256(bytes);
}
