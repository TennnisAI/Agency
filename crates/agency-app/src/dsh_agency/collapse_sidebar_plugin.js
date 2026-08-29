// Host plugin Agency mounts via `dsh web --patch`. dsh's layout store is
// transient and starts with the conversation sidebar open (280px); there is no
// setting or flag for a collapsed default. We inject a one-shot script that
// clicks the Collapse control once it mounts. Stop watching as soon as the
// sidebar is collapsed (or already was): keeping the observer would collapse
// again the next time the user opens it.
export const name = 'agency-collapse-sidebar'
export function apply(ctx) {
  ctx.on('webserver/index-inject', (table) => {
    table.push({
      kind: 'script',
      placement: 'body',
      text: `(function () {
  var collapse = ["Collapse sidebar", "收起侧边栏"];
  var open = ["Open sidebar", "打开侧边栏"];
  function has(labels) {
    for (var i = 0; i < labels.length; i++) {
      if (document.querySelector('button[aria-label="' + labels[i] + '"]')) return true;
    }
    return false;
  }
  function collapseNow() {
    for (var i = 0; i < collapse.length; i++) {
      var btn = document.querySelector('button[aria-label="' + collapse[i] + '"]');
      if (btn) { btn.click(); return true; }
    }
    return false;
  }
  function done() {
    if (collapseNow()) return true;
    return has(open);
  }
  if (done()) return;
  var obs = new MutationObserver(function () {
    if (done()) obs.disconnect();
  });
  obs.observe(document.documentElement, { childList: true, subtree: true });
  setTimeout(function () { obs.disconnect(); }, 20000);
})();`,
    })
  })
}
