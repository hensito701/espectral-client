/**
 * Tests for the NBT servers.dat codec in src/engine/nbt.mjs.
 *
 * The module is a *minimal servers.dat codec*, not a generic NBT codec:
 *   - parseServersDat(buf)  -> ServerEntry[] (decodes root TAG_Compound{servers: TAG_List<compound>})
 *   - writeServersDat(list) -> Buffer (always modern 1.21.2+ format)
 * The reader internally supports byte/short/int/long/float/double/string/list/
 * compound/byte-array/int-array; it does NOT support long-array (0x0c).
 * There is no generic encoder, so "round-trip" coverage is exercised through
 * writeServersDat -> parseServersDat plus hand-built wire payloads for reader
 * type coverage. All behaviors below were verified against the real module.
 */
import { describe, test } from 'node:test';
import assert from 'node:assert/strict';
import * as nbt from '../src/engine/nbt.mjs';

const {
  TAG_END,
  TAG_BYTE,
  TAG_SHORT,
  TAG_INT,
  TAG_LONG,
  TAG_FLOAT,
  TAG_DOUBLE,
  TAG_BYTE_ARRAY,
  TAG_STRING,
  TAG_LIST,
  TAG_COMPOUND,
  TAG_INT_ARRAY,
  parseServersDat,
  writeServersDat,
} = nbt;

// ---------------------------------------------------------------------------
// Wire-format helpers (mirror the module's own byte layout)
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

function nbtString(str) {
  const b = Buffer.from(String(str), 'utf8');
  return Buffer.concat([u16(b.length), b]);
}

const val = {
  byte: (v) => Buffer.from([v & 0xff]),
  short: (v) => {
    const b = Buffer.alloc(2);
    b.writeInt16BE(v);
    return b;
  },
  int: (v) => {
    const b = Buffer.alloc(4);
    b.writeInt32BE(v);
    return b;
  },
  long: (v) => {
    const b = Buffer.alloc(8);
    b.writeBigInt64BE(v);
    return b;
  },
  float: (v) => {
    const b = Buffer.alloc(4);
    b.writeFloatBE(v);
    return b;
  },
  double: (v) => {
    const b = Buffer.alloc(8);
    b.writeDoubleBE(v);
    return b;
  },
  string: (v) => nbtString(v),
  list: (elemType, elems) => Buffer.concat([Buffer.from([elemType]), i32(elems.length), ...elems]),
  compound: (pairs) =>
    Buffer.concat([...pairs.flatMap(([t, name, bytes]) => [Buffer.from([t]), nbtString(name), bytes]), Buffer.from([TAG_END])]),
  byteArray: (bytes) => Buffer.concat([i32(bytes.length), Buffer.from(bytes)]),
  intArray: (ints) => Buffer.concat([i32(ints.length), ...ints.map((v) => val.int(v))]),
};

/** One TAG_Compound with the given [type, name, valueBytes] fields. */
const entry = (pairs) => val.compound(pairs);

/** A servers.dat-shaped entry compound, the way the writer emits it. */
function serverEntry({ name = '', ip = '', hidden = false, accept_textures = false, icon } = {}) {
  const pairs = [
    [TAG_BYTE, 'hidden', val.byte(hidden ? 1 : 0)],
    [TAG_STRING, 'ip', val.string(ip)],
    [TAG_STRING, 'name', val.string(name)],
  ];
  if (icon !== undefined) pairs.push([TAG_STRING, 'icon', val.string(icon)]);
  pairs.push([TAG_BYTE, 'acceptTextures', val.byte(accept_textures ? 1 : 0)]);
  return entry(pairs);
}

/** Full modern payload: root compound {servers: list}, optional legacy prefix byte and root name. */
function payload({ rootName = '', legacy = 0, servers = [] } = {}) {
  const root = Buffer.concat([
    Buffer.from([TAG_COMPOUND]),
    nbtString(rootName),
    Buffer.concat([Buffer.from([TAG_LIST]), nbtString('servers'), Buffer.from([TAG_COMPOUND]), i32(servers.length), ...servers]),
    Buffer.from([TAG_END]),
  ]);
  return legacy ? Buffer.concat([Buffer.from([legacy]), root]) : root;
}

const DEFAULT_ENTRY = { name: '', ip: '', hidden: false, accept_textures: false, has_icon: false };

// ---------------------------------------------------------------------------
// Public API surface
// ---------------------------------------------------------------------------

describe('public API surface', () => {
  test('exports exactly the documented constants and functions', () => {
    assert.deepEqual(
      Object.keys(nbt).sort(),
      [
        'TAG_BYTE', 'TAG_BYTE_ARRAY', 'TAG_COMPOUND', 'TAG_DOUBLE', 'TAG_END',
        'TAG_FLOAT', 'TAG_INT', 'TAG_INT_ARRAY', 'TAG_LIST', 'TAG_LONG',
        'TAG_SHORT', 'TAG_STRING', 'parseServersDat', 'writeServersDat',
      ].sort(),
    );
  });

  test('TAG constants match the NBT wire format', () => {
    assert.equal(TAG_END, 0x00);
    assert.equal(TAG_BYTE, 0x01);
    assert.equal(TAG_SHORT, 0x02);
    assert.equal(TAG_INT, 0x03);
    assert.equal(TAG_LONG, 0x04);
    assert.equal(TAG_FLOAT, 0x05);
    assert.equal(TAG_DOUBLE, 0x06);
    assert.equal(TAG_BYTE_ARRAY, 0x07);
    assert.equal(TAG_STRING, 0x08);
    assert.equal(TAG_LIST, 0x09);
    assert.equal(TAG_COMPOUND, 0x0a);
    assert.equal(TAG_INT_ARRAY, 0x0b);
    // long-array (0x0c) is deliberately NOT part of the module's API;
    // the reader rejects it (see "reader type coverage").
    assert.equal(nbt.TAG_LONG_ARRAY, undefined);
  });
});

// ---------------------------------------------------------------------------
// Golden byte vectors (produced by the module's own encoder)
// ---------------------------------------------------------------------------

describe('writeServersDat golden bytes', () => {
  test('empty list encodes to the exact byte sequence', () => {
    assert.equal(
      writeServersDat([]).toString('hex'),
      '0a0000090007736572766572730a0000000000',
    );
  });

  test('single entry encodes to the exact byte sequence', () => {
    assert.equal(
      writeServersDat([{ name: 'Test', ip: 'localhost', hidden: false, accept_textures: false }]).toString('hex'),
      '0a0000090007736572766572730a00000001' +
        '01000668696464656e00' +
        '080002697000096c6f63616c686f7374' +
        '0800046e616d65000454657374' +
        '08000469636f6e0000' +
        '01000e616363657074546578747572657300' +
        '00' +
        '00',
    );
  });
});

// ---------------------------------------------------------------------------
// write -> parse round-trip (self-consistency)
// ---------------------------------------------------------------------------

describe('write → parse round-trip', () => {
  test('empty payload round-trips to an empty server list', () => {
    assert.deepEqual(parseServersDat(writeServersDat([])), []);
  });

  test('a full entry round-trips to the identical structure', () => {
    const input = [{ name: 'Test', ip: 'localhost', hidden: true, accept_textures: true, has_icon: false }];
    const out = parseServersDat(writeServersDat(input));
    assert.deepEqual(out, [
      { name: 'Test', ip: 'localhost', hidden: true, accept_textures: true, has_icon: false },
    ]);
  });

  test('multiple entries with unicode and empty strings round-trip', () => {
    const input = [
      { name: 'español 😀', ip: 'mc.example.com', hidden: true, accept_textures: false, has_icon: false },
      { name: '', ip: '', hidden: false, accept_textures: true, has_icon: false },
      { name: 'A'.repeat(60000), ip: 'x', hidden: true, accept_textures: true, has_icon: false },
    ];
    const out = parseServersDat(writeServersDat(input));
    assert.deepEqual(
      out.map((e) => [e.name, e.ip, e.hidden, e.accept_textures]),
      input.map((e) => [e.name, e.ip, e.hidden, e.accept_textures]),
    );
    assert.equal(out[0].name, 'español 😀');
  });

  test('has_icon is never written; icon always decodes as "" (documented)', () => {
    // The module doc states ServerEntry has no icon content (only has_icon),
    // so a written icon is always ''. Therefore has_icon cannot survive a round-trip.
    const out = parseServersDat(writeServersDat([{ name: 'x', ip: 'y', has_icon: true }]));
    assert.deepEqual(out, [{ name: 'x', ip: 'y', hidden: false, accept_textures: false, has_icon: false }]);
  });

  test('a name longer than 65535 utf8 bytes throws ERR_OUT_OF_RANGE on write', () => {
    assert.throws(() => writeServersDat([{ name: 'x'.repeat(70000) }]), {
      name: 'RangeError',
      code: 'ERR_OUT_OF_RANGE',
    });
  });
});

// ---------------------------------------------------------------------------
// Reader type coverage (hand-built wire payloads; no generic encoder exists)
// ---------------------------------------------------------------------------

describe('parseServersDat reader type coverage', () => {
  test('every supported NBT type decodes inside an entry compound, including boundaries', () => {
    const servers = [
      entry([
        [TAG_BYTE, 'b', val.byte(-128)],
        [TAG_BYTE, 'B', val.byte(127)],
        [TAG_SHORT, 's', val.short(-32768)],
        [TAG_SHORT, 'S', val.short(32767)],
        [TAG_INT, 'i', val.int(-2147483648)],
        [TAG_INT, 'I', val.int(2147483647)],
        [TAG_LONG, 'l', val.long(-9223372036854775808n)],
        [TAG_LONG, 'L', val.long(9223372036854775807n)],
        [TAG_FLOAT, 'f', val.float(0.5)],
        [TAG_DOUBLE, 'd', val.double(Math.PI)],
        [TAG_STRING, 'emptyStr', val.string('')],
        [TAG_LIST, 'emptyList', val.list(TAG_COMPOUND, [])],
        [TAG_LIST, 'intList', val.list(TAG_INT, [val.int(1), val.int(-2)])],
        [TAG_COMPOUND, 'nested', val.compound([[TAG_STRING, 'inner', val.string('x')]])],
        [TAG_BYTE_ARRAY, 'ba', val.byteArray([0, 127, 255])],
        [TAG_INT_ARRAY, 'ia', val.intArray([-2147483648, 2147483647])],
        [TAG_STRING, 'name', val.string('ABC')],
        [TAG_BYTE, 'hidden', val.byte(1)],
      ]),
    ];
    const out = parseServersDat(payload({ servers }));
    assert.deepEqual(out, [{ name: 'ABC', ip: '', hidden: true, accept_textures: false, has_icon: false }]);
  });

  test('long-array (0x0c) is not supported and throws', () => {
    const buf = payload({ servers: [entry([[0x0c, 'la', Buffer.alloc(0)]])] });
    assert.throws(() => parseServersDat(buf), /unsupported NBT tag 0xc at offset \d+/);
  });

  test('float NaN and Infinity decode without error', () => {
    // NaN / ±Infinity decode to plain Number values; entryFromCompound only maps
    // name/ip/hidden/accept_textures/has_icon, so these fields are dropped and
    // the entry comes out as DEFAULT_ENTRY.
    const buf = payload({
      servers: [entry([
        [TAG_FLOAT, 'f', val.float(NaN)],
        [TAG_FLOAT, 'negInf', val.float(-Infinity)],
        [TAG_DOUBLE, 'd', val.double(Infinity)],
        [TAG_DOUBLE, 'negInfD', val.double(-Infinity)],
      ])],
    });
    assert.deepEqual(parseServersDat(buf), [DEFAULT_ENTRY]);
  });

  test('decoded primitive values are observable through the hidden field (=== 1 mapping)', () => {
    // entryFromCompound maps hidden: c.hidden === 1 || c.hidden === true.
    // byte/short/int/float/double 1 decode to Number 1 -> true; long decodes to
    // BigInt (1n !== 1) -> false; NaN is never === 1 -> false.
    const mk = (type, bytes) => payload({ servers: [entry([[type, 'hidden', bytes]])] });
    const withHidden = (hidden) => ({ ...DEFAULT_ENTRY, hidden });
    assert.deepEqual(parseServersDat(mk(TAG_BYTE, val.byte(1))), [withHidden(true)]);
    assert.deepEqual(parseServersDat(mk(TAG_SHORT, val.short(1))), [withHidden(true)]);
    assert.deepEqual(parseServersDat(mk(TAG_INT, val.int(1))), [withHidden(true)]);
    assert.deepEqual(parseServersDat(mk(TAG_FLOAT, val.float(1.0))), [withHidden(true)]);
    assert.deepEqual(parseServersDat(mk(TAG_DOUBLE, val.double(1.0))), [withHidden(true)]);
    assert.deepEqual(parseServersDat(mk(TAG_LONG, val.long(1n))), [withHidden(false)]);
    assert.deepEqual(parseServersDat(mk(TAG_BYTE, val.byte(2))), [withHidden(false)]);
    assert.deepEqual(parseServersDat(mk(TAG_BYTE, val.byte(-1))), [withHidden(false)]);
    assert.deepEqual(parseServersDat(mk(TAG_FLOAT, val.float(NaN))), [withHidden(false)]);
    assert.deepEqual(parseServersDat(mk(TAG_DOUBLE, val.double(NaN))), [withHidden(false)]);
  });
});

// ---------------------------------------------------------------------------
// Named vs unnamed root compound
// ---------------------------------------------------------------------------

describe('root compound naming', () => {
  test('named and unnamed root compounds parse identically (root name is consumed and ignored)', () => {
    const unnamed = payload({ servers: [serverEntry({ name: 'A', ip: 'b' })] });
    const named = payload({ rootName: 'hello', servers: [serverEntry({ name: 'A', ip: 'b' })] });
    assert.deepEqual(parseServersDat(named), parseServersDat(unnamed));
    assert.deepEqual(parseServersDat(named), [{ name: 'A', ip: 'b', hidden: false, accept_textures: false, has_icon: false }]);
    assert.deepEqual(parseServersDat(payload({ rootName: 'servers' })), []);
  });
});

// ---------------------------------------------------------------------------
// Legacy format (< 1.21.2)
// ---------------------------------------------------------------------------

describe('legacy format', () => {
  test('0x02 and 0x03 prefix bytes are detected and skipped', () => {
    const base = writeServersDat([{ name: 'A', ip: 'b', hidden: true, accept_textures: true }]);
    const modern = parseServersDat(base);
    assert.deepEqual(parseServersDat(Buffer.concat([Buffer.from([0x02]), base])), modern);
    assert.deepEqual(parseServersDat(Buffer.concat([Buffer.from([0x03]), base])), modern);
  });

  test('legacy entries missing modern fields decode with defaults', () => {
    const buf = payload({
      legacy: 0x02,
      servers: [entry([[TAG_STRING, 'name', val.string('Old')], [TAG_STRING, 'ip', val.string('1.2.3.4')]])],
    });
    assert.deepEqual(parseServersDat(buf), [
      { name: 'Old', ip: '1.2.3.4', hidden: false, accept_textures: false, has_icon: false },
    ]);
  });

  test('an entry with an icon string decodes has_icon: true (parser honors icon; writer never emits it)', () => {
    const buf = payload({
      legacy: 0x03,
      servers: [entry([[TAG_STRING, 'name', val.string('N')], [TAG_STRING, 'icon', val.string('data:image/png;base64,AAAA')]])],
    });
    assert.deepEqual(parseServersDat(buf), [
      { name: 'N', ip: '', hidden: false, accept_textures: false, has_icon: true },
    ]);
  });
});

// ---------------------------------------------------------------------------
// Truncated / garbage / nonstandard payloads (actual module behavior)
// ---------------------------------------------------------------------------

describe('truncated and garbage buffers', () => {
  test('empty buffer throws', () => {
    assert.throws(() => parseServersDat(Buffer.alloc(0)), /not a servers\.dat NBT payload \(first byte end-of-buffer\)/);
  });

  test('a first byte that is not TAG_Compound throws', () => {
    assert.throws(() => parseServersDat(Buffer.from([0x7f])), /not a servers\.dat NBT payload \(first byte 0x7f\)/);
    assert.throws(() => parseServersDat(Buffer.from([0x09, 0x00, 0x00])), /first byte 0x9/);
  });

  test('a legacy prefix followed by garbage throws', () => {
    assert.throws(() => parseServersDat(Buffer.from([0x02, 0x7f])), /first byte 0x7f/);
    assert.throws(() => parseServersDat(Buffer.from([0x02])), /first byte end-of-buffer/);
  });

  test('a bare root compound ending at EOF (no TAG_End) decodes to an empty list', () => {
    assert.deepEqual(parseServersDat(Buffer.from([0x0a, 0x00, 0x00])), []);
    // the trailing root TAG_End is optional when EOF is hit first
    const full = writeServersDat([]);
    assert.deepEqual(parseServersDat(full.subarray(0, full.length - 1)), []);
  });

  test('an entry compound whose trailing TAG_End is cut off at EOF still decodes fully', () => {
    // strip the root TAG_End AND the last entry's TAG_End: the entry's final
    // field then ends exactly at EOF, which readCompound tolerates.
    const full = writeServersDat([{ name: 'Ok', ip: 'h', hidden: true, accept_textures: true }]);
    const stripped = full.subarray(0, full.length - 2);
    assert.deepEqual(parseServersDat(stripped), [
      { name: 'Ok', ip: 'h', hidden: true, accept_textures: true, has_icon: false },
    ]);
  });

  test('truncation mid-field throws ERR_OUT_OF_RANGE once the reader reads past EOF', () => {
    // root compound, empty root name, then a lone field type byte at EOF
    assert.throws(() => parseServersDat(Buffer.from([0x0a, 0x00, 0x00, 0x01])), {
      name: 'RangeError',
      code: 'ERR_OUT_OF_RANGE',
    });
    // valid root + list header, entry cut short inside a string field
    const buf = Buffer.concat([
      Buffer.from([TAG_COMPOUND]),
      u16(0),
      Buffer.concat([Buffer.from([TAG_LIST]), nbtString('servers'), Buffer.from([TAG_COMPOUND]), i32(1)]),
      Buffer.from([TAG_STRING]),
      Buffer.concat([u16(4), Buffer.from('na')]),
    ]);
    assert.throws(() => parseServersDat(buf), { name: 'RangeError', code: 'ERR_OUT_OF_RANGE' });
  });

  test('a string whose content runs past EOF decodes as a partial string without throwing', () => {
    // Hand-built so the payload ends exactly at 'Te' (no entry/root TAG_End
    // follow it): the reader's buf.toString clamps the slice to the available
    // bytes, so the name decodes as the partial 'Te' and no throw occurs.
    const buf = Buffer.concat([
      Buffer.from([TAG_COMPOUND]),
      u16(0),
      Buffer.concat([Buffer.from([TAG_LIST]), nbtString('servers'), Buffer.from([TAG_COMPOUND]), i32(1)]),
      Buffer.from([TAG_STRING]),
      nbtString('name'),
      u16(4),
      Buffer.from('Te'),
    ]);
    assert.deepEqual(parseServersDat(buf), [{ ...DEFAULT_ENTRY, name: 'Te' }]);
  });

  test('a garbage string length overrunning EOF decodes as "" without throwing', () => {
    // Length 60000 but the payload ends right after the length bytes: the
    // reader clamps the slice to EOF (empty), so the name decodes as "" and no
    // throw occurs.
    const buf = Buffer.concat([
      Buffer.from([TAG_COMPOUND]),
      u16(0),
      Buffer.concat([Buffer.from([TAG_LIST]), nbtString('servers'), Buffer.from([TAG_COMPOUND]), i32(1)]),
      Buffer.from([TAG_STRING]),
      nbtString('name'),
      u16(60000),
    ]);
    assert.deepEqual(parseServersDat(buf), [DEFAULT_ENTRY]);
  });

  test('a list with a negative count decodes as an empty list', () => {
    const buf = Buffer.concat([
      Buffer.from([TAG_COMPOUND]),
      u16(0),
      Buffer.concat([Buffer.from([TAG_LIST]), nbtString('servers'), Buffer.from([TAG_COMPOUND]), i32(-5)]),
      Buffer.from([TAG_END]),
    ]);
    assert.deepEqual(parseServersDat(buf), []);
  });

  test('a non-empty list whose element type is TAG_End throws', () => {
    const buf = Buffer.concat([
      Buffer.from([TAG_COMPOUND]),
      u16(0),
      Buffer.concat([Buffer.from([TAG_LIST]), nbtString('servers'), Buffer.from([TAG_END]), i32(1)]),
      Buffer.from([TAG_END]),
    ]);
    assert.throws(() => parseServersDat(buf), /unsupported NBT tag 0x0 at offset \d+/);
  });

  test('a non-list servers field is treated as an empty list', () => {
    const buf = Buffer.concat([
      Buffer.from([TAG_COMPOUND]),
      u16(0),
      Buffer.concat([Buffer.from([TAG_COMPOUND]), nbtString('servers'), val.compound([])]),
      Buffer.from([TAG_END]),
    ]);
    assert.deepEqual(parseServersDat(buf), []);
  });

  test('a servers list of non-compound elements still maps to one default entry each', () => {
    const buf = Buffer.concat([
      Buffer.from([TAG_COMPOUND]),
      u16(0),
      Buffer.concat([Buffer.from([TAG_LIST]), nbtString('servers'), Buffer.from([TAG_INT]), i32(2), val.int(1), val.int(2)]),
      Buffer.from([TAG_END]),
    ]);
    assert.deepEqual(parseServersDat(buf), [DEFAULT_ENTRY, DEFAULT_ENTRY]);
  });
});

// ---------------------------------------------------------------------------
// Input coercion
// ---------------------------------------------------------------------------

describe('input coercion', () => {
  test('Uint8Array and plain Array inputs are accepted (Buffer.from coercion)', () => {
    const buf = writeServersDat([{ name: 'A', ip: 'b', hidden: true, accept_textures: true }]);
    const expected = parseServersDat(buf);
    assert.deepEqual(parseServersDat(new Uint8Array(buf)), expected);
    assert.deepEqual(parseServersDat([...buf]), expected);
  });
});
