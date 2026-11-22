// End-to-end test of the hub binary, using the same socket.io v2 client as the web frontend.
// Run: NODE_PATH=../frontend/node_modules node tests/e2e/e2e.js target/debug/vtally-hub   (needs `npm install` in ../frontend)
const io = require('socket.io-client')
const dgram = require('dgram')
const net = require('net')
const { spawn } = require('child_process')
const assert = require('assert')

const HTTP = 3055, UDP = 7455
const bin = process.argv[2]
const server = spawn(bin, ['--with-test', '--port', String(HTTP), '--tally-port', String(UDP), '--frontend-dir', '/nonexistent'], { stdio: ['ignore', 'inherit', 'inherit'] })
const sleep = ms => new Promise(r => setTimeout(r, ms))

const log = []
function waitFor(socket, event, pred = () => true, timeout = 3000) {
  return new Promise((resolve, reject) => {
    const t = setTimeout(() => { socket.off(event, h); reject(new Error(`timeout waiting for ${event}`)) }, timeout)
    const h = (...args) => { if (pred(...args)) { clearTimeout(t); socket.off(event, h); resolve(args[0]) } }
    socket.on(event, h)
  })
}

async function main() {
  await sleep(800)
  const socket = io(`http://localhost:${HTTP}`)
  await new Promise((res, rej) => { socket.on('connect', res); socket.on('connect_error', rej) })
  console.log('connected with socket.io v2 client, id', socket.id)

  // --- snapshots on subscribe
  const mixer = waitFor(socket, 'mixer.state')
  const program = waitFor(socket, 'program.state')
  const channel = waitFor(socket, 'channel.state')
  const tally = waitFor(socket, 'tally.state')
  const cfgAtem = waitFor(socket, 'config.state.atem')
  const cfgMixer = waitFor(socket, 'config.state.mixer')
  const cfgTally = waitFor(socket, 'config.state.tallyconfig')
  const logState = waitFor(socket, 'tally.log.state')
  for (const t of ['mixer', 'program', 'channel', 'tally', 'config', 'tallyLog']) socket.emit(`events.${t}.subscribe`)
  console.log('mixer.state', JSON.stringify(await mixer))
  console.log('program.state', JSON.stringify(await program))
  console.log('channel.state', JSON.stringify(await channel))
  assert.deepStrictEqual((await tally).tallies, [])
  assert.deepStrictEqual(await cfgAtem, { ip: '127.0.0.1', port: 9910 })
  const m = await cfgMixer
  console.log('config.state.mixer', JSON.stringify(m))
  assert.strictEqual(m.mixerName, 'test')
  assert.deepStrictEqual(m.allowedMixers, ['test', 'null', 'atem', 'obs', 'rolandV8HD', 'rolandV60HD', 'vmix'])
  assert.strictEqual((await cfgTally).stBrightness, 100)
  await logState

  // --- the test mixer is driven from outside (this is what cypress does)
  let p = waitFor(socket, 'program.state', s => s.programs && s.programs[0] === '1')
  socket.emit('config.change.test', { programs: ['1'], previews: ['2'] }, 'test')
  console.log('program.state after config.change.test', JSON.stringify(await p))

  // --- a UDP tally appears
  const udp = dgram.createSocket('udp4')
  const received = []
  udp.on('message', msg => received.push(msg.toString()))
  await new Promise(r => udp.bind(0, '127.0.0.1', r))
  let t = waitFor(socket, 'tally.state', s => s.tallies.some(x => x.name === 'Cam 1' && x.state === 1))
  udp.send('tally-ho "Cam 1"', UDP, '127.0.0.1')
  const st = (await t).tallies[0]
  console.log('tally.state', JSON.stringify(st))
  assert.strictEqual(st.type, 'udp'); assert.strictEqual(st.address, '127.0.0.1'); assert.strictEqual(st.port, udp.address().port)

  // patch it -> it receives program color
  t = waitFor(socket, 'tally.state', s => s.tallies[0].channelId === '1')
  socket.emit('tally.patch', 'Cam 1', 'udp', '1')
  await t
  await sleep(300)
  console.log('udp datagrams received by tally:', new Set(received))
  assert(received.includes('O255/000/000 S255/000/000'), 'tally was told it is on air')

  // switch program -> preview color
  received.length = 0
  socket.emit('config.change.test', { programs: ['3'], previews: ['1'] }, 'test')
  await sleep(400)
  assert(received.includes('O000/255/000 S000/255/000'), 'tally was told it is in preview')

  // tally settings override
  received.length = 0
  socket.emit('tally.settings', 'Cam 1', 'udp', { stBrightness: 50, stPreview: false })
  await sleep(300)
  assert(received.includes('O000/255/000 S000/000/000'), 'settings applied: ' + [...new Set(received)])

  // highlight
  received.length = 0
  socket.emit('tally.highlight', 'Cam 1', 'udp')
  await sleep(300)
  assert(received.includes('O255/255/255 S128/128/128 0xAA 125'), 'highlight: ' + [...new Set(received)])

  // keepalives
  received.length = 0
  await sleep(1500)
  assert(received.length >= 8, `keepalives (${received.length} in 1.5s)`)

  // tally log
  const logP = waitFor(socket, 'tally.log', l => l.log.message === 'hello from tally')
  udp.send('log "Cam 1" WARN "hello from tally"', UDP, '127.0.0.1')
  console.log('tally.log', JSON.stringify(await logP))

  // missing / disconnected (test mode: 1s / 3s)
  await waitFor(socket, 'tally.state', s => s.tallies[0].state === 2, 4000)
  console.log('tally went MISSING')
  await waitFor(socket, 'tally.state', s => s.tallies[0].state === 0, 5000)
  console.log('tally went DISCONNECTED')

  // --- web tally
  t = waitFor(socket, 'tally.state', s => s.tallies.some(x => x.name === 'Phone'))
  socket.emit('tally.create', 'Phone', '3')
  console.log('web tally created', JSON.stringify((await t).tallies.find(x => x.name === 'Phone')))
  const web = io(`http://localhost:${HTTP}`)
  await new Promise(r => web.on('connect', r))
  const ws = waitFor(web, 'webTally.state')
  web.emit('events.webTally.subscribe', 'Phone')
  const wst = await ws
  console.log('webTally.state', JSON.stringify(wst))
  assert.strictEqual(wst.command, 'on-air')
  assert.strictEqual(wst.tally.connectedClients.length, 1)
  console.log('  browser address as seen by hub:', wst.tally.connectedClients[0].address)
  const next = waitFor(web, 'webTally.state', s => s.command === 'release')
  socket.emit('config.change.test', { programs: ['1'], previews: [] }, 'test')
  console.log('webTally.state after program change', JSON.stringify(await next))

  const invalid = waitFor(web, 'webTally.invalid')
  web.emit('events.webTally.subscribe', 'Nope')
  assert.strictEqual(await invalid, 'Nope')
  console.log('invalid web tally rejected')

  const disc = waitFor(socket, 'tally.state', s => s.tallies.find(x => x.name === 'Phone').connectedClients.length === 0)
  web.close()
  await disc
  console.log('web tally browser disconnect noticed')

  // --- config: tally defaults + switching to the null mixer
  const defs = waitFor(socket, 'config.state.tallyconfig', c => c.opColor === 'yellow-pink')
  socket.emit('config.change.tallyconfig', { opColor: 'yellow-pink', stBrightness: 70 })
  console.log('config.state.tallyconfig', JSON.stringify(await defs))
  const nul = waitFor(socket, 'program.state', s => s.programs === null)
  const chP = waitFor(socket, 'channel.state', c => c.channels.length === 8)
  const mixDisc = waitFor(socket, 'mixer.state', s => s.isConnected === false)
  const selP = waitFor(socket, 'config.state.mixer', c => c.mixerName === 'null')
  socket.emit('config.change.null', 'null')
  await nul; await mixDisc; await selP
  const ch = await chP
  console.log('switched to null mixer: programs null, channels reset to', ch.channels.length)

  // --- vMix: a fake vMix TCP API server
  const vmixXml = '<vmix><inputs><input number="1" shortTitle="Wide"/><input number="2" shortTitle="Close"/><input number="3" shortTitle="Titles"/></inputs></vmix>'
  const vmix = net.createServer(sck => {
    sck.write('VERSION OK 24.0.0.1\r\n')
    let buf = ''
    sck.on('data', d => {
      buf += d
      let i
      while ((i = buf.indexOf('\r\n')) >= 0) {
        const cmd = buf.slice(0, i); buf = buf.slice(i + 2)
        if (cmd === 'SUBSCRIBE TALLY') sck.write('SUBSCRIBE OK TALLY Subscribed\r\nTALLY OK 120\r\n')
        else if (cmd === 'XML') sck.write(`XML ${vmixXml.length}\r\n${vmixXml}\r\n`)
      }
    })
    sck.on('error', () => {})
  })
  await new Promise(r => vmix.listen(0, '127.0.0.1', r))
  const vmixPort = vmix.address().port
  const vmixProg = waitFor(socket, 'program.state', s => s.programs && s.programs[0] === '1' && s.previews[0] === '2')
  const vmixCh = waitFor(socket, 'channel.state', c => c.channels.length === 3 && c.channels[2].name === 'Titles')
  const vmixConn = waitFor(socket, 'mixer.state', s => s.isConnected === true)
  socket.emit('config.change.vmix', { ip: '127.0.0.1', port: vmixPort }, 'vmix')
  console.log('vMix program.state', JSON.stringify(await vmixProg))
  console.log('vMix channels', JSON.stringify((await vmixCh).channels))
  await vmixConn
  console.log('vMix is connected')
  vmix.close()

  // --- ATEM: a fake ATEM that speaks the UDP protocol
  const atem = dgram.createSocket('udp4')
  await new Promise(r => atem.bind(0, '127.0.0.1', r))
  const atemPort = atem.address().port
  const cmdRecord = (name, data) => { const b = Buffer.alloc(8 + data.length); b.writeUInt16BE(b.length, 0); b.write(name, 4, 'ascii'); Buffer.from(data).copy(b, 8); return b }
  const packet = (flags, session, id, payload) => { const b = Buffer.alloc(12 + payload.length); b.writeUInt16BE((flags << 11) | b.length, 0); b.writeUInt16BE(session, 2); b.writeUInt16BE(id, 10); payload.copy(b, 12); return b }
  const inpr = (id, name) => { const b = Buffer.alloc(36); b.writeUInt16BE(id, 0); b.write(name, 2); return cmdRecord('InPr', b) }
  const dump = [
    Buffer.concat([cmdRecord('_ver', [0, 2, 0, 28]), inpr(1, 'Cam A'), inpr(2, 'Cam B'), inpr(3, 'Cam C')]),
    Buffer.concat([cmdRecord('PrgI', [0, 0, 0, 3]), cmdRecord('PrvI', [0, 0, 0, 2, 0, 0, 0, 0]), cmdRecord('InCm', [1, 0, 0, 0])]),
  ]
  let acked = 0
  let client
  atem.on('message', (msg, rinfo) => {
    const flags = msg[0] >> 3
    if (flags & 2) { // hello
      client = rinfo
      atem.send(packet(2, 0x8042, 0, Buffer.from([2, 0, 0, 0, 0, 0, 0, 0])), rinfo.port, rinfo.address)
      dump.forEach((p, i) => atem.send(packet(1, 0x8042, i + 1, p), rinfo.port, rinfo.address))
    } else if (flags & 16) acked = Math.max(acked, msg.readUInt16BE(4))
  })
  const atemProg = waitFor(socket, 'program.state', s => s.programs && s.programs[0] === '3' && s.previews[0] === '2')
  const atemCh = waitFor(socket, 'channel.state', c => c.channels.length === 3 && c.channels[0].name === 'Cam A')
  const atemConn = waitFor(socket, 'mixer.state', s => s.isConnected === true)
  socket.emit('config.change.atem', { ip: '127.0.0.1', port: atemPort }, 'atem')
  console.log('ATEM program.state', JSON.stringify(await atemProg))
  console.log('ATEM channels', JSON.stringify((await atemCh).channels))
  await atemConn
  await sleep(100)
  assert.strictEqual(acked, 2, 'the ATEM packets were acknowledged')
  console.log('ATEM is connected and acknowledged', acked, 'packets')
  // a cut
  const cut = waitFor(socket, 'program.state', s => s.programs && s.programs[0] === '1')
  atem.send(packet(1, 0x8042, 3, cmdRecord('PrgI', [0, 0, 0, 1])), client.port, client.address)
  console.log('ATEM program after a cut', JSON.stringify(await cut))
  atem.close()

  // --- remove
  t = waitFor(socket, 'tally.state', s => !s.tallies.some(x => x.name === 'Phone'))
  socket.emit('tally.remove', 'Phone', 'web')
  await t
  console.log('removed web tally')

  console.log('\nALL E2E CHECKS PASSED')
}
main().then(() => { server.kill('SIGTERM'); process.exit(0) }, e => { console.error('E2E FAILED:', e); server.kill('SIGTERM'); process.exit(1) })
