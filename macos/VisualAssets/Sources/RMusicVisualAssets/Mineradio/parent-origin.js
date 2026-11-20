/* R-Music local visual host. A message must come from the exact owning parent window. */
function rmusicTrustedParent(event) {
  if (event.source !== parent) return false;
  if (event.origin === location.origin) return true;
  // WKWebView serializes local file message origins differently across its frame boundaries.
  return location.protocol === 'file:' && (event.origin === 'null' || event.origin === 'file://');
}
