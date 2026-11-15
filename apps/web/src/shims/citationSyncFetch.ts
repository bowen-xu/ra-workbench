function unsupportedSyncFetch(): never {
  throw new Error("Citation.js synchronous URL fetching is disabled in the browser bundle");
}

export default Object.assign(unsupportedSyncFetch, { Headers: globalThis.Headers });
