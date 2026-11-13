// The UI server's one control proxy, shared by the routes it answers on.
import { readFile } from 'node:fs/promises';
import { REN_CONTROL_LOGINS, REN_CONTROL_TOKEN_FILE, REN_GATEWAY_URL } from '$app/env/private';
import { RunsAccessConfig, RunsProxy } from './control.ts';

export const proxy = new RunsProxy(
  RunsAccessConfig.access(REN_CONTROL_TOKEN_FILE, REN_CONTROL_LOGINS),
  REN_GATEWAY_URL,
  { fetch, readToken: (path) => readFile(path, 'utf8') },
);
