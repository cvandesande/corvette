// Refuses a malformed activity query the way the activity route does, so a stub answers 400 like the server.

const refuseMalformedActivityQuery = (route) => {
  const refuse = (message) => {
    route.fulfill({ status: 400, json: { code: "bad_request", message } });
    return true;
  };
  const url = route.request().url();
  const query = url.slice(url.indexOf("?") + 1);
  if (query === "") {
    return false;
  }
  const seenNames = [];
  for (const segment of query.split("&")) {
    if (segment === "") {
      return refuse("empty query parameter");
    }
    const equals = segment.indexOf("=");
    if (equals === -1) {
      return refuse(`query parameter \`${decodeURIComponent(segment)}\` is missing its value`);
    }
    const name = decodeURIComponent(segment.slice(0, equals));
    if (!["after", "before", "cameras", "include"].includes(name)) {
      return refuse(`unknown query parameter \`${name}\``);
    }
    if (seenNames.includes(name)) {
      return refuse(`repeated query parameter \`${name}\``);
    }
    seenNames.push(name);
  }
  return false;
};

module.exports = { refuseMalformedActivityQuery };
