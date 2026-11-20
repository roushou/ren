import test from 'node:test';
import assert from 'node:assert/strict';
import {
  AttachmentId,
  Frame,
  Gallery,
  Strip,
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

test('a frame keeps its image whole, sized from its shape between bounds', () => {
  // A desktop screenshot: as wide as its shape asks, up to the widest.
  assert.deepEqual(Frame.SENT.fit({ width: 2560, height: 1440 }), {
    frame: { width: 213, height: 120 },
    image: { width: 213, height: 120 },
  });
  // A phone screenshot: narrow, but never under the narrowest frame.
  assert.deepEqual(Frame.SENT.fit({ width: 390, height: 844 }), {
    frame: { width: 72, height: 120 },
    image: { width: 55, height: 120 },
  });
  // A banner: the widest frame, the image fitted inside it, not cropped.
  assert.deepEqual(Frame.SENT.fit({ width: 1600, height: 120 }), {
    frame: { width: 280, height: 120 },
    image: { width: 280, height: 21 },
  });
  // An icon smaller than the frame: shown at its own size, not enlarged.
  assert.deepEqual(Frame.SENT.fit({ width: 48, height: 48 }), {
    frame: { width: 120, height: 120 },
    image: { width: 48, height: 48 },
  });
  // The tray's frames follow the same rule at their own size.
  assert.deepEqual(Frame.TRAY.fit({ width: 2560, height: 1440 }).frame, { width: 71, height: 40 });
  // Unknown dimensions: a square frame, nothing sized inside it.
  assert.deepEqual(Frame.TRAY.fit({ width: 0, height: 0 }), {
    frame: { width: 40, height: 40 },
    image: { width: 0, height: 0 },
  });
});

test('a strip shows the first images and counts the rest, never just one', () => {
  const strip = (count: number) => {
    const made = new Strip(Array.from({ length: count }, (_, index) => index));
    return { shown: made.shown.length, rest: made.rest };
  };
  assert.deepEqual(strip(0), { shown: 0, rest: 0 });
  assert.deepEqual(strip(4), { shown: 4, rest: 0 });
  assert.deepEqual(strip(5), { shown: 5, rest: 0 });
  assert.deepEqual(strip(9), { shown: 4, rest: 5 });
});

test("a prompt's images count their places by position, skipping the gone", () => {
  const other: AttachmentRead = { ...read, name: 'other.png', sha256: 'cd' };
  const sent = Gallery.sent([read, other, read], (one) =>
    one === other ? null : `/a/${one.sha256}`,
  );
  assert.deepEqual(sent, [
    { kind: 'kept', image: { src: '/a/ab', read }, view: 0 },
    { kind: 'gone', read: other },
    // The same image sent twice is a place of its own.
    { kind: 'kept', image: { src: '/a/ab', read }, view: 1 },
  ]);
});
