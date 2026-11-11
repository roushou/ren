import { readFile } from 'node:fs/promises';
import { REN_CONTROL_LOGINS, REN_CONTROL_TOKEN_FILE, REN_GATEWAY_URL } from '$app/env/private';
import { RunsAccessConfig, RunsProxy } from '#lib/server/control.ts';
import type { RequestHandler } from './$types';

const proxy = new RunsProxy(
  RunsAccessConfig.access(REN_CONTROL_TOKEN_FILE, REN_CONTROL_LOGINS),
  REN_GATEWAY_URL,
  { fetch, readToken: (path) => readFile(path, 'utf8') },
);

export const GET: RequestHandler = ({ request }) => proxy.respond(request);
export const POST = GET;
