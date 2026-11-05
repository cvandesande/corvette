// Refuses a malformed clip-link query the way the clip-link route does, so a stub answers 400 like the server.

const refuseMalformedClipLinkQuery = (route) => {
  const refuse = (message) => {
    route.fulfill({ status: 400, json: { code: "bad_request", message } });
    return true;
  };
  const url = route.request().url();
  const query = url.slice(url.indexOf("?") + 1);
  const params = new Map();
  if (query !== "") {
    for (const segment of query.split("&")) {
      if (segment === "") {
        return refuse("empty query parameter");
      }
      const equals = segment.indexOf("=");
      if (equals === -1) {
        return refuse(`query parameter \`${decodeURIComponent(segment)}\` is missing its value`);
      }
      const name = decodeURIComponent(segment.slice(0, equals));
      if (!["camera", "start", "end"].includes(name)) {
        return refuse(`unknown query parameter \`${name}\``);
      }
      if (params.has(name)) {
        return refuse(`repeated query parameter \`${name}\``);
      }
      params.set(name, decodeURIComponent(segment.slice(equals + 1)));
    }
  }
  for (const name of ["camera", "start", "end"]) {
    if (!params.has(name)) {
      return refuse(`missing query parameter \`${name}\``);
    }
  }
  if (params.get("camera") === "") {
    return refuse("`camera` must name one camera");
  }
  const values = [];
  for (const name of ["start", "end"]) {
    if (!/^[+-]?(\d+(\.\d*)?|\.\d+)([eE][+-]?\d+)?$/.test(params.get(name))) {
      return refuse(`\`${name}\` must be a finite number of seconds, at least 0`);
    }
    const value = Number(params.get(name));
    if (!Number.isFinite(value) || value < 0) {
      return refuse(`\`${name}\` must be a finite number of seconds, at least 0`);
    }
    values.push(value);
  }
  const [start, end] = values;
  if (start >= end) {
    return refuse("`end` must be greater than `start`");
  }
  if (end - start > 2678400) {
    return refuse("`end` is more than 2678400 seconds after `start`");
  }
  return false;
};

module.exports = { refuseMalformedClipLinkQuery };
