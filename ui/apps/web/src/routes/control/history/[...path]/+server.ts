import { proxy } from '#lib/server/runsProxy.ts';
import type { RequestHandler } from './$types';

export const POST: RequestHandler = ({ request }) => proxy.respond(request);
