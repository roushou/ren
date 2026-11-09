/**
 * The dev slot's traffic controls, reached by the dev toolbar through the UI
 * server. This is the one place the UI forwards a change, and it exists only
 * when the dev slot sets `TGV_DEV_TRAFFIC_URL`: the target is the slot's fake
 * traffic generator on loopback, never the daemon, and nothing here touches
 * recorded history or inference.
 */

/** The toolbar's requests and where each goes. */
const routes: Record<string, { method: 'GET' | 'POST'; target: string }> = {
  '/dev/traffic/status': { method: 'GET', target: '/api/status' },
  '/dev/traffic/settings': { method: 'POST', target: '/api/settings' },
  '/dev/traffic/burst': { method: 'POST', target: '/api/burst' },
};
const maxBody = 64 * 1024;

export async function devTrafficResponse(
  origin: string | undefined,
  request: Request,
  fetcher: typeof fetch = fetch,
): Promise<Response> {
  const headers = { 'cache-control': 'no-store', 'x-content-type-options': 'nosniff' };
  const route = routes[new URL(request.url).pathname];
  if (!origin || !route) return new Response(null, { status: 404, headers });
  if (request.method !== route.method) {
    return new Response(null, { status: 405, headers: { ...headers, allow: route.method } });
  }
  let body: string | undefined;
  if (route.method === 'POST') {
    if (request.headers.get('content-type') !== 'application/json') {
      return new Response(null, { status: 415, headers });
    }
    body = await request.text();
    if (body.length > maxBody) return new Response(null, { status: 413, headers });
  }
  try {
    // Global fetch: no browser cookies or credentials are forwarded.
    const upstream = await fetcher(new URL(route.target, origin), {
      method: route.method,
      headers: body === undefined ? {} : { 'content-type': 'application/json' },
      body,
      redirect: 'manual',
      signal: AbortSignal.any([request.signal, AbortSignal.timeout(30_000)]),
    });
    return new Response(upstream.body, {
      status: upstream.status,
      headers: {
        ...headers,
        'content-type': upstream.headers.get('content-type') ?? 'text/plain',
      },
    });
  } catch {
    return Response.json(
      { error: 'The dev traffic generator is not running.' },
      { status: 502, headers },
    );
  }
}
