import { proxy } from '#lib/server/runsProxy.ts';
import type { RequestHandler } from './$types';

// The Release view's Promote (`/control/release/promote`).
export const POST: RequestHandler = ({ request }) => proxy.respond(request);
