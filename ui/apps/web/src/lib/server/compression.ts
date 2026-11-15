/**
 * Gzip for the pages this server renders. A page carries its rendered rows
 * and the data it hydrates from, often hundreds of KiB, and over a tailnet
 * that wait is the first paint. The built scripts and styles are already
 * served precompressed; data responses stay as they are, since a compressor
 * holds back the pieces a navigation's streamed reads arrive in.
 */
export class HtmlCompression {
  /** Whether a response of `contentType`, not yet encoded, should be gzipped
   * for a request that accepts `acceptEncoding`. */
  static wanted(
    acceptEncoding: string | null,
    contentType: string | null,
    contentEncoding: string | null,
  ): boolean {
    if (contentEncoding !== null) return false;
    if (!contentType?.toLowerCase().startsWith('text/html')) return false;
    return HtmlCompression.accepts(acceptEncoding, 'gzip');
  }

  /** Whether `header` (an Accept-Encoding) accepts `coding`, by name or `*`,
   * with a quality above zero. */
  static accepts(header: string | null, coding: string): boolean {
    if (!header) return false;
    let star: boolean | null = null;
    for (const part of header.split(',')) {
      const [name = '', ...params] = part.trim().toLowerCase().split(';');
      const q = params.map((param) => param.trim()).find((param) => param.startsWith('q='));
      const allowed = q === undefined || Number(q.slice(2)) > 0;
      if (name.trim() === coding) return allowed;
      if (name.trim() === '*') star = allowed;
    }
    return star ?? false;
  }

  /** `response`, gzipped when `request` accepts it and it is a page. */
  static apply(request: Request, response: Response): Response {
    if (
      !response.body ||
      !HtmlCompression.wanted(
        request.headers.get('accept-encoding'),
        response.headers.get('content-type'),
        response.headers.get('content-encoding'),
      )
    )
      return response;
    const headers = new Headers(response.headers);
    headers.set('content-encoding', 'gzip');
    headers.delete('content-length');
    headers.append('vary', 'accept-encoding');
    return new Response(response.body.pipeThrough(new CompressionStream('gzip')), {
      status: response.status,
      statusText: response.statusText,
      headers,
    });
  }
}
