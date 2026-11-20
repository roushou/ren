import { proxy } from '#lib/server/runsProxy.ts';
import type { RequestHandler } from './$types';

// Asking for a preview of a commit, and withdrawing the ask
// (`/control/previews/ask`, `/control/previews/withdraw`).
export const POST: RequestHandler = ({ request }) => proxy.respond(request);
