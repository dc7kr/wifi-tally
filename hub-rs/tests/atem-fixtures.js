// Generates tests/fixtures/atem-visible-inputs.json: random sequences of ATEM commands together
// with the tallies that the reference implementation (the `atem-connection` library, which the
// Node hub uses) derives from them. The Rust tests replay the sequences and compare.
//
//   npm install atem-connection@3.4.0   (in any scratch directory)
//   NODE_PATH=<scratch>/node_modules node tests/atem-fixtures.js > tests/fixtures/atem-visible-inputs.json
const { CommandParser } = require('atem-connection/dist/lib/atemCommandParser')
const { AtemStateUtil } = require('atem-connection/dist/state')
const { listVisibleInputs } = require('atem-connection/dist/lib/tally')
const { InternalPortType } = require('atem-connection/dist/enums')

// protocol 8.0 and the version before it
const VERSIONS = [131100, 131099]
let VERSION

// a small deterministic random number generator
let seed = 20240607
const rnd = () => { seed = (seed * 1664525 + 1013904223) >>> 0; return seed / 4294967296 }
const int = (n) => Math.floor(rnd() * n)
const pick = (list) => list[int(list.length)]

const SOURCES = [0, 1, 2, 3, 4, 5, 6, 7, 1000, 2001, 3010, 3011, 3020, 6000, 10010, 10011, 10020, 10021]
const bool = () => (rnd() < 0.5 ? 1 : 0)

function buf(size, fn) {
  const b = Buffer.alloc(size)
  fn(b)
  return b
}

const generators = {
  InPr: () => buf(36, (b) => {
    const id = pick(SOURCES)
    b.writeUInt16BE(id, 0)
    b.write(pick(['Cam 1', 'Cam 2', 'Bars', 'Müller', 'ME 1', 'SuperSource']), 2)
    b.writeUInt8(id === 6000 ? 6 : id >= 10000 ? 128 : id >= 1000 ? pick([2, 4, 5]) : id === 0 ? 1 : 0, 32)
  }),
  PrgI: () => buf(4, (b) => { b.writeUInt8(int(2), 0); b.writeUInt16BE(pick(SOURCES), 2) }),
  PrvI: () => buf(8, (b) => { b.writeUInt8(int(2), 0); b.writeUInt16BE(pick(SOURCES), 2) }),
  TrSS: () => buf(8, (b) => {
    b.writeUInt8(int(2), 0); b.writeUInt8(int(5), 1); b.writeUInt8(int(32), 2)
    b.writeUInt8(int(5), 3); b.writeUInt8(int(32), 4)
  }),
  TrPs: () => buf(8, (b) => {
    b.writeUInt8(int(2), 0); b.writeUInt8(bool(), 1); b.writeUInt8(int(30), 2); b.writeUInt16BE(pick([0, 0, 500, 10000]), 4)
  }),
  TrPr: () => buf(4, (b) => { b.writeUInt8(int(2), 0); b.writeUInt8(bool(), 1) }),
  TDpP: () => buf(4, (b) => { b.writeUInt8(int(2), 0); b.writeUInt8(25, 1); b.writeUInt16BE(pick(SOURCES), 2) }),
  TDvP: () => buf(20, (b) => {
    b.writeUInt8(int(2), 0); b.writeUInt8(pick([0, 1, 8, 17, 32, 33, 34]), 3)
    b.writeUInt16BE(pick(SOURCES), 4); b.writeUInt16BE(pick(SOURCES), 6); b.writeUInt8(bool(), 8)
  }),
  TWpP: () => buf(20, (b) => {
    b.writeUInt8(int(2), 0); b.writeUInt16BE(pick([0, 0, 100]), 4); b.writeUInt16BE(pick(SOURCES), 6)
  }),
  TStP: () => buf(20, (b) => { b.writeUInt8(int(2), 0); b.writeUInt8(int(2), 1) }),
  KeOn: () => buf(4, (b) => { b.writeUInt8(int(2), 0); b.writeUInt8(int(4), 1); b.writeUInt8(bool(), 2) }),
  KeBP: () => buf(24, (b) => {
    b.writeUInt8(int(2), 0); b.writeUInt8(int(4), 1); b.writeUInt8(int(4), 2)
    b.writeUInt16BE(pick(SOURCES), 6); b.writeUInt16BE(pick(SOURCES), 8)
  }),
  DskS: () => buf(8, (b) => { b.writeUInt8(int(2), 0); b.writeUInt8(bool(), 1); b.writeUInt8(bool(), 2) }),
  DskP: () => buf(20, (b) => { b.writeUInt8(int(2), 0); b.writeUInt8(bool(), 1) }),
  DskB: () => buf(8, (b) => { b.writeUInt8(int(2), 0); b.writeUInt16BE(pick(SOURCES), 2); b.writeUInt16BE(pick(SOURCES), 4) }),
  SSrc: () => buf(36, (b) => {
    if (VERSION >= 131100) { b.writeUInt16BE(pick(SOURCES), 2); b.writeUInt16BE(pick(SOURCES), 4); b.writeUInt8(int(2), 6) }
    else { b.writeUInt16BE(pick(SOURCES), 0); b.writeUInt16BE(pick(SOURCES), 2); b.writeUInt8(int(2), 4) }
  }),
  SSBP: () => buf(24, (b) => {
    if (VERSION >= 131100) { b.writeUInt8(0, 0); b.writeUInt8(int(4), 1); b.writeUInt8(bool(), 2); b.writeUInt16BE(pick(SOURCES), 4) }
    else { b.writeUInt8(int(4), 0); b.writeUInt8(bool(), 1); b.writeUInt16BE(pick(SOURCES), 2) }
  }),
}
const names = Object.keys(generators)

const parser = new CommandParser()

function newState() {
  const state = AtemStateUtil.Create()
  state.info.apiVersion = VERSION
  state.info.capabilities = { mixEffects: 2, downstreamKeyers: 2, superSources: 1, DVEs: 1 }
  state.info.mixEffects = [{ keyCount: 4 }, { keyCount: 4 }]
  return state
}

const cases = []
for (let c = 0; c < 300; c++) {
  VERSION = VERSIONS[c % VERSIONS.length]
  parser.version = VERSION
  const state = newState()
  const commands = []
  const count = 1 + int(40)
  for (let i = 0; i < count; i++) {
    const name = pick(names)
    const payload = generators[name]()
    const ctor = parser.commandFromRawName(name)
    let applied = true
    try {
      ctor.deserialize(payload, VERSION).applyToState(state)
    } catch (e) {
      applied = false // the library refuses it; the generator should not produce such commands
      throw e
    }
    commands.push([name, payload.toString('hex')])
  }
  const sorted = (list) => list.map((i) => i.toString()).sort()
  cases.push({
    version: VERSION,
    commands,
    program: [0, 1].map((me) => listVisibleInputs('program', state, me)),
    preview: [0, 1].map((me) => listVisibleInputs('preview', state, me)),
    channels: Object.values(state.inputs)
      .filter((i) => i && i.internalPortType === InternalPortType.External)
      .map((i) => [i.inputId, i.longName]),
  })
}
console.log(JSON.stringify({ cases }))
