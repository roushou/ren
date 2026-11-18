// How a process exited (the Rust `ProcessExit`), as the control API sends a
// run's end and a login's: its kind, and the value a code or a signal
// carries. Parsed here at the boundary, once for every reader.
import { isRecord, ShapeError, wire, type Decoder, type StrayFields } from './decode.ts';
import type { ProcessExit as WireProcessExit } from './wire/ProcessExit.ts';

/** The kinds of exit, pinned to `ren-recorder`'s `capture::ExitKind`. */
export const exitKinds = ['code', 'signal', 'unknown'] as const;
export type ExitKind = (typeof exitKinds)[number];

export type ProcessExit =
  { kind: 'code'; value: number } | { kind: 'signal'; value: number } | { kind: 'unknown' };

const integer: Decoder<number, number> = (value, path) => {
  if (typeof value !== 'number' || !Number.isInteger(value))
    throw new ShapeError(path, 'an integer');
  return value;
};

/** An exit; a kind this UI does not know reads as an unknown exit. */
export const processExit: Decoder<ProcessExit, WireProcessExit> = (value, path) => {
  if (!isRecord(value)) throw new ShapeError(path, 'an object');
  switch (value.kind) {
    case 'code':
    case 'signal':
      return { kind: value.kind, value: integer(value.value, `${path}.value`) };
    default:
      return { kind: 'unknown' };
  }
};
wire<StrayFields<Extract<ProcessExit, { kind: 'code' }>, WireProcessExit>>();
