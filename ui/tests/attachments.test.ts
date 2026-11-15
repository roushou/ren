import test from 'node:test';
import assert from 'node:assert/strict';
import {
  AttachmentId,
  Gallery,
  Refusals,
  Tray,
  attachmentRead,
  keptList,
  type AttachmentRead,
} from '../src/lib/attachments.ts';

const id = AttachmentId.parse('00000000-0000-0000-0000-0000000000a1')!;
const read: AttachmentRead = {
  name: 'shot.png',
  format: 'png',
  width: 640,
  height: 480,
  size: 2048,
  sha256: 'ab',
};

test('an attachment reads as the daemon writes it, keeping an unknown format', () => {
  const written = {
    name: 'shot.avif',
    kind: { kind: 'image', format: 'avif', dimensions: { width: 3, height: 2 } },
    size: 10,
    sha256: 'cd',
  };
  assert.deepEqual(attachmentRead(written, ''), {
    name: 'shot.avif',
    format: 'avif',
    width: 3,
    height: 2,
    size: 10,
    sha256: 'cd',
  });
  const listed = keptList(
    { attachments: [{ id: id.toUpperCase(), read: written, sent: true }] },
    '',
  );
  assert.equal(listed.attachments[0]!.id, id);
  assert.equal(listed.attachments[0]!.sent, true);
  assert.throws(() => keptList({ attachments: [{ id: 'nope', read: written, sent: false }] }, ''));
});

test('the tray keeps uploads in order and sends only the ready ones', () => {
  const first = new Tray().adding('a.png');
  const second = first.tray.adding('b.txt');
  let tray = second.tray;
  assert.equal(tray.uploading(), true);
  tray = tray.uploaded(first.key, id, read).refused(second.key, 'Not an image.');
  assert.equal(tray.uploading(), false);
  assert.deepEqual(tray.ids(), [id]);
  assert.deepEqual(
    tray.chips.map((chip) => chip.state),
    ['ready', 'refused'],
  );
  // Sent: the ready one leaves; the refused one stays to be read, then goes.
  const sent = tray.sent();
  assert.deepEqual(
    sent.chips.map((chip) => chip.state),
    ['refused'],
  );
  assert.equal(sent.without(second.key).chips.length, 0);
  // Keys never repeat, so a late answer cannot land on a newer chip.
  assert.notEqual(sent.adding('c.png').key, second.key);
});

test('a sent image is found by its content, and described by its size', () => {
  const gallery = new Gallery([{ id, read, sent: true }]);
  assert.equal(gallery.find(read), id);
  assert.equal(gallery.find({ ...read, sha256: 'ef' }), null);
  assert.equal(
    Gallery.url('0190f3a0-0000-7000-8000-0000000000aa', id),
    `/control/sessions/0190f3a0-0000-7000-8000-0000000000aa/attachments/${id}`,
  );
  assert.equal(Gallery.describe(read), '640×480 · 2 KB');
  assert.equal(Gallery.describe({ ...read, size: 3 * 1024 * 1024 }), '640×480 · 3.0 MB');
  assert.equal(Gallery.takes('image/webp'), true);
  assert.equal(Gallery.takes('image/svg+xml'), false);
});

test('refusals are worded for the operator', () => {
  assert.equal(Refusals.say('attachment_unrecognized'), 'Not a PNG, JPEG, GIF, or WebP image.');
  assert.equal(Refusals.say('no_images'), 'This client does not take images.');
  assert.equal(Refusals.say('something_new'), 'Refused: something_new.');
});
