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
