import test from 'node:test';
import assert from 'node:assert/strict';
import { GatewayPlace } from '../src/lib/place.ts';

test('a tailnet address shows its node and masks the tailnet name', () => {
  const place = GatewayPlace.parse('https://ren.tail1234ab.ts.net:8443');
  assert.equal(place.network, 'tailnet');
  assert.equal(place.label, 'ren · tailnet');
  assert.equal(place.masked, 'https://ren.•••.ts.net:8443');
  assert.equal(
    place.mask('ren run claude --gateway https://ren.tail1234ab.ts.net:8443'),
    'ren run claude --gateway https://ren.•••.ts.net:8443',
  );
  const ip = GatewayPlace.parse('http://100.101.102.103:8787');
  assert.equal(ip.label, 'tailnet');
  assert.equal(ip.masked, 'http://•••:8787');
});

test('loopback and private addresses need no mask', () => {
  for (const origin of ['http://127.0.0.1:8787', 'http://localhost:8787', 'http://[::1]:8787']) {
    const place = GatewayPlace.parse(origin);
    assert.equal(place.network, 'loopback', origin);
    assert.equal(place.label, 'this machine');
    assert.equal(place.maskable, false);
  }
  const lan = GatewayPlace.parse('http://192.168.1.20:8787');
  assert.equal(lan.label, 'local network');
  assert.equal(lan.maskable, false);
  assert.equal(GatewayPlace.parse('http://nas.local:8787').label, 'nas · local network');
  assert.equal(GatewayPlace.parse('http://devbox:8787').label, 'devbox · local network');
});

test('a public address is flagged and keeps only its first label', () => {
  const place = GatewayPlace.parse('https://gateway.example.com');
  assert.equal(place.network, 'public');
  assert.equal(place.label, 'gateway · public');
  assert.equal(place.masked, 'https://gateway.•••');
  assert.equal(GatewayPlace.parse('http://203.0.113.9:8787').masked, 'http://•••:8787');
  const broken = GatewayPlace.parse('not a url');
  assert.equal(broken.network, 'public');
  assert.equal(broken.masked, GatewayPlace.MASK);
});
