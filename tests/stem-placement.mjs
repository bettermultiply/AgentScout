import test from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from '../webui/node_modules/vite/dist/node/index.js'
const server = await createServer({ root: new URL('../webui', import.meta.url).pathname, server: { middlewareMode: true }, appType: 'custom' })
const { placeStems } = await server.ssrLoadModule('/src/tree/stemPlacement.ts')
test.after(() => server.close())
const stem = { id: 's', distance: 20, range: 10, preferred: 1, base: 20, angles: [10, 20, 30] }
const place = (s = stem, n = []) => placeStems([s], n).get(s.id)
test('no neighbors retains preferred side and nearest base', () => assert.deepEqual(place(), { side: 1, tilt: 20 }))
test('same psi child switches side', () => assert.deepEqual(place(stem, [{distance:20,side:1,angleDeg:70}]), {side:-1,tilt:20}))
test('two sides choose greatest minimum clearance', () => assert.deepEqual(place(stem, [{distance:20,side:1,angleDeg:70},{distance:20,side:-1,angleDeg:50}]), {side:-1,tilt:10}))
test('fixed mode changes side but not angle', () => assert.deepEqual(place({...stem,angles:[20]},[{distance:20,side:1,angleDeg:70}]),{side:-1,tilt:20}))
test('neighbors beyond S are ignored', () => assert.deepEqual(place(stem,[{distance:31,side:1,angleDeg:70}]),{side:1,tilt:20}))
test('placement is deterministic and sorts by distance', () => {
 const a={...stem,id:'a',distance:0}, b={...stem,id:'b',distance:5}
 assert.deepEqual(placeStems([b,a],[]),placeStems([a,b],[]))
 assert.deepEqual(placeStems([b,a],[]),placeStems([b,a],[]))
})
