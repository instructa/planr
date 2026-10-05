// Runs only for /assets/media/*. Static assets answer a byte range with the whole file, and Safari then
// refuses to play the video, so this Worker cuts the requested range out of the asset.
export default {
  async fetch(request, env) {
    const response = await env.ASSETS.fetch(request);
    const headers = new Headers(response.headers);
    headers.set("accept-ranges", "bytes");
    const range = /^bytes=(\d*)-(\d*)$/.exec(request.headers.get("range") ?? "");
    if (response.status !== 200 || !range || (range[1] === "" && range[2] === "")) {
      return new Response(response.body, { status: response.status, headers });
    }
    const body = await response.arrayBuffer();
    const size = body.byteLength;
    const start = range[1] === "" ? Math.max(0, size - Number(range[2])) : Number(range[1]);
    const end = range[1] === "" || range[2] === "" ? size - 1 : Math.min(Number(range[2]), size - 1);
    if (start > end) {
      return new Response(null, { status: 416, headers: { "content-range": `bytes */${size}` } });
    }
    headers.set("content-range", `bytes ${start}-${end}/${size}`);
    headers.set("content-length", String(end - start + 1));
    return new Response(body.slice(start, end + 1), { status: 206, headers });
  },
};
