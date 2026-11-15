export function gatewayOrigin(value: string): string {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error('Gateway URL must be an absolute HTTP(S) origin.');
  }
  if (
    !['http:', 'https:'].includes(url.protocol) ||
    url.username ||
    url.password ||
    url.pathname !== '/' ||
    url.search ||
    url.hash
  ) {
    throw new Error(
      'Gateway URL must be an HTTP(S) origin without credentials, a path, query, or fragment.',
    );
  }
  return url.origin;
}

/**
 * The dev slot's traffic control origin (`cargo xtask dev`): loopback HTTP
 * only, since the UI server forwards the dev toolbar's requests there.
 */
export function devTrafficOrigin(value: string): string {
  const origin = gatewayOrigin(value);
  const url = new URL(origin);
  if (url.protocol !== 'http:' || !['127.0.0.1', '[::1]', 'localhost'].includes(url.hostname)) {
    throw new Error('The dev traffic URL must be a loopback HTTP origin.');
  }
  return origin;
}
