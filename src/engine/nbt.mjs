/**
 * Minimal NBT codec for Minecraft servers.dat (parse + write).
 *
 * Modern format (1.21.2+): root TAG_Compound(0x0a) containing a TAG_List(0x09)
 * named 'servers' whose elements are TAG_Compounds with:
 *   hidden         TAG_Byte(0x01)
 *   ip             TAG_String(0x08)
 *   name           TAG_String(0x08)
 *   icon           TAG_String(0x08)
 *   acceptTextures TAG_Byte(0x01)
 * String/list lengths are u16/u32 big-endian; every compound ends with
 * TAG_End(0x00). List counts are i32 big-endian.
 *
 * Legacy (<1.21.2): same root shape but entries carry only {name, ip, icon?}
 * (icon optional) and the payload may be prefixed by a version byte 0x02/0x03.
 * Detection: first byte 0x0a (no prefix) vs 0x02/0x03 (skip one byte, then 0x0a).
 *
 * writeServersDat always emits the modern format (v0.1.0 targets 1.21.11).
 * ServerEntry has no icon content (only `has_icon`), so a written icon is ''.
 */
export const TAG_END = 0x00;
export const TAG_BYTE = 0x01;
export const TAG_SHORT = 0x02;
export const TAG_INT = 0x03;
export const TAG_LONG = 0x04;
export const TAG_FLOAT = 0x05;
export const TAG_DOUBLE = 0x06;
export const TAG_BYTE_ARRAY = 0x07;
export const TAG_STRING = 0x08;
export const TAG_LIST = 0x09;
export const TAG_COMPOUND = 0x0a;
export const TAG_INT_ARRAY = 0x0b;

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

function readName(buf, state) {
  const len = buf.readUInt16BE(state.offset);
  state.offset += 2;
  const s = buf.toString('utf8', state.offset, state.offset + len);
  state.offset += len;
  return s;
}

function readString(buf, state) {
  const len = buf.readUInt16BE(state.offset);
  state.offset += 2;
  const s = buf.toString('utf8', state.offset, state.offset + len);
  state.offset += len;
  return s;
}

function readCompound(buf, state) {
  const out = {};
  while (state.offset < buf.length) {
    const type = buf[state.offset++];
    if (type === TAG_END) break;
    const name = readName(buf, state);
    out[name] = readValue(buf, state, type);
  }
  return out;
}

function readValue(buf, state, type) {
  switch (type) {
    case TAG_BYTE: {
      const v = buf.readInt8(state.offset);
      state.offset += 1;
      return v;
    }
    case TAG_SHORT: {
      const v = buf.readInt16BE(state.offset);
      state.offset += 2;
      return v;
    }
    case TAG_INT: {
      const v = buf.readInt32BE(state.offset);
      state.offset += 4;
      return v;
    }
    case TAG_LONG: {
      const v = buf.readBigInt64BE(state.offset);
      state.offset += 8;
      return v;
    }
    case TAG_FLOAT: {
      const v = buf.readFloatBE(state.offset);
      state.offset += 4;
      return v;
    }
    case TAG_DOUBLE: {
      const v = buf.readDoubleBE(state.offset);
      state.offset += 8;
      return v;
    }
    case TAG_STRING:
      return readString(buf, state);
    case TAG_LIST: {
      const elemType = buf[state.offset++];
      const count = buf.readInt32BE(state.offset);
      state.offset += 4;
      const arr = [];
      for (let i = 0; i < count; i++) arr.push(readValue(buf, state, elemType));
      return arr;
    }
    case TAG_COMPOUND:
      return readCompound(buf, state);
    case TAG_BYTE_ARRAY: {
      const n = buf.readInt32BE(state.offset);
      state.offset += 4;
      const v = buf.subarray(state.offset, state.offset + n);
      state.offset += n;
      return [...v];
    }
    case TAG_INT_ARRAY: {
      const n = buf.readInt32BE(state.offset);
      state.offset += 4;
      const v = [];
      for (let i = 0; i < n; i++) v.push(buf.readInt32BE(state.offset + i * 4));
      state.offset += n * 4;
      return v;
    }
    default:
      throw new Error(`unsupported NBT tag 0x${type.toString(16)} at offset ${state.offset}`);
  }
}

/**
 * Parse a servers.dat payload into ServerEntry[].
 * Detects the legacy 0x02/0x03 prefix byte; entries missing modern fields are
 * filled with defaults.
 */
export function parseServersDat(buf) {
  if (!Buffer.isBuffer(buf)) buf = Buffer.from(buf);
  let offset = 0;
  if (buf.length > 0 && (buf[0] === 0x02 || buf[0] === 0x03)) offset = 1;
  if (buf.length <= offset || buf[offset] !== TAG_COMPOUND) {
    const seen = buf.length > offset ? `0x${buf[offset].toString(16)}` : 'end-of-buffer';
    throw new Error(`not a servers.dat NBT payload (first byte ${seen})`);
  }
  const state = { offset };
  state.offset++; // consume the root TAG_Compound type byte
  readName(buf, state); // consume the root name ("" in servers.dat, but part of the payload)
  const root = readCompound(buf, state);
  const list = Array.isArray(root.servers) ? root.servers : [];
  return list.map(entryFromCompound);
}

function entryFromCompound(c) {
  const icon = typeof c.icon === 'string' ? c.icon : '';
  return {
    name: typeof c.name === 'string' ? c.name : '',
    ip: typeof c.ip === 'string' ? c.ip : '',
    hidden: c.hidden === 1 || c.hidden === true,
    accept_textures: c.acceptTextures === 1 || c.acceptTextures === true,
    has_icon: icon.length > 0,
  };
}

// ---------------------------------------------------------------------------
// Writer (modern 1.21.2+ format)
// ---------------------------------------------------------------------------

function u16(n) {
  const b = Buffer.alloc(2);
  b.writeUInt16BE(n);
  return b;
}

function i32(n) {
  const b = Buffer.alloc(4);
  b.writeInt32BE(n);
  return b;
}

function nbtString(s) {
  const b = Buffer.from(String(s), 'utf8');
  return Buffer.concat([u16(b.length), b]);
}

/** Serialize one ServerEntry as a modern compound (all 5 fields, TAG_End). */
function serverEntryCompound(e) {
  const parts = [
    Buffer.from([TAG_BYTE]),
    nbtString('hidden'),
    Buffer.from([e.hidden ? 1 : 0]),
    Buffer.from([TAG_STRING]),
    nbtString('ip'),
    nbtString(e.ip ?? ''),
    Buffer.from([TAG_STRING]),
    nbtString('name'),
    nbtString(e.name ?? ''),
    Buffer.from([TAG_STRING]),
    nbtString('icon'),
    nbtString(''), // ServerEntry carries no icon bytes, only has_icon
    Buffer.from([TAG_BYTE]),
    nbtString('acceptTextures'),
    Buffer.from([e.accept_textures ? 1 : 0]),
    Buffer.from([TAG_END]),
  ];
  return Buffer.concat(parts);
}

/** Serialize ServerEntry[] into a modern-format servers.dat payload. */
export function writeServersDat(entries) {
  const list = Buffer.concat([
    Buffer.from([TAG_LIST]),
    nbtString('servers'),
    Buffer.from([TAG_COMPOUND]),
    i32(entries.length),
    ...entries.map(serverEntryCompound),
  ]);
  return Buffer.concat([Buffer.from([TAG_COMPOUND]), u16(0), list, Buffer.from([TAG_END])]);
}
