import { legacyRedirects } from './redirects.mjs';

const redirects = new Map(
  legacyRedirects.map(({ source, destination }) => [source, destination]),
);

const contentTypes = new Map([
  ['/api/search', 'application/json; charset=utf-8'],
  ['/llms.txt', 'text/markdown; charset=utf-8'],
  ['/llms-full.txt', 'text/markdown; charset=utf-8'],
]);

const worker = {
  async fetch(request, env) {
    const url = new URL(request.url);
    const destination = redirects.get(url.pathname);

    if (destination) {
      return new Response(null, {
        status: 308,
        headers: {
          location: `${destination}${url.search}`,
          'x-planr-edge': 'legacy-redirect',
        },
      });
    }

    const response = await env.ASSETS.fetch(request);
    const contentType = url.pathname.endsWith('.md')
      ? 'text/markdown; charset=utf-8'
      : contentTypes.get(url.pathname);

    if (!contentType) return response;

    const headers = new Headers(response.headers);
    headers.set('content-type', contentType);
    headers.set('x-planr-edge', 'agent-asset');
    return new Response(response.body, {
      status: response.status,
      statusText: response.statusText,
      headers,
    });
  },
};

export default worker;
