const id = '[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}';
const readablePath = new RegExp(
  `^/(?:health|api/(?:metrics|storage|calls|sessions|usage|live|calls/${id}(?:/body/(?:request|response))?|sessions/${id}/calls))$`,
);
const livePath = '/api/live';
const eventId = /^\d{1,20}$/;
/**
 * An SSE comment, which clients ignore. The Node server sends response headers
 * only with the first body chunk, so without it EventSource would not report
 * the feed open until the daemon's first event or keep-alive.
 */
const openingComment = new TextEncoder().encode(': connected\n\n');

/** The live feed, prefixed with the opening comment. Reads are pulled, so backpressure holds. */
function liveBody(upstream: ReadableStream<Uint8Array>): ReadableStream<Uint8Array> {
  const reader = upstream.getReader();
  return new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(openingComment);
    },
    async pull(controller) {
      const { done, value } = await reader.read();
      if (done) controller.close();
      else controller.enqueue(value);
    },
    cancel(reason) {
      return reader.cancel(reason);
    },
  });
}

/** Only the daemon's read-only management surface is reachable through the UI. */
export async function managementResponse(
  origin: string,
  request: Request,
  fetcher: typeof fetch = fetch,
): Promise<Response> {
  const url = new URL(request.url);
  const headers = { 'cache-control': 'no-store', 'x-content-type-options': 'nosniff' };
  if (!readablePath.test(url.pathname)) return new Response(null, { status: 404, headers });
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    return new Response(null, { status: 405, headers: { ...headers, allow: 'GET, HEAD' } });
  }
  // The live feed is long-lived: it ends when the browser leaves or the daemon stops.
  const live = url.pathname === livePath;
  const timeout = AbortSignal.timeout(10_000);
  const resume = request.headers.get('last-event-id');
  const upstreamHeaders: Record<string, string> = live
    ? {
        accept: 'text/event-stream',
        ...(resume && eventId.test(resume) ? { 'last-event-id': resume } : {}),
      }
    : { accept: 'application/json' };
  try {
    // Use global fetch, not event.fetch: never inherit browser cookies or credentials.
    const upstream = await fetcher(new URL(url.pathname + url.search, origin), {
      method: request.method,
      headers: upstreamHeaders,
      redirect: 'manual',
      signal: live ? request.signal : AbortSignal.any([request.signal, timeout]),
    });
    if (upstream.status >= 300 && upstream.status < 400) {
      await upstream.body?.cancel();
      return Response.json({ error: 'Unexpected daemon redirect.' }, { status: 502, headers });
    }
    // The body streams through as it arrives; nothing here buffers it.
    const body = live && upstream.ok && upstream.body ? liveBody(upstream.body) : upstream.body;
    return new Response(body, {
      status: upstream.status,
      headers: {
        ...headers,
        'content-type':
          upstream.headers.get('content-type') ?? (live ? 'text/event-stream' : 'application/json'),
      },
    });
  } catch {
    return Response.json(
      { error: 'Cannot reach the TGV daemon.' },
      { status: !live && timeout.aborted ? 504 : 502, headers },
    );
  }
}
