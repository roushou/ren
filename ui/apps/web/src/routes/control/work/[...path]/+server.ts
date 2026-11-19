import { proxy } from '#lib/server/runsProxy.ts';
import type { RequestHandler } from './$types';

// Work items: queued (`/control/work`) and moved on (`/control/work/{id}/…`).
export const POST: RequestHandler = ({ request }) => proxy.respond(request);
