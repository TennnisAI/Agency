/* The only JavaScript on the site beyond the inline pre-paint theme snippet:
   the theme toggle, and the download page's one-line non-Mac notice. */
(function () {
  var btn = document.getElementById('theme-toggle');
  if (btn) {
    btn.addEventListener('click', function () {
      var root = document.documentElement;
      var explicit = root.getAttribute('data-theme');
      var system = window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
      var next = (explicit || system) === 'dark' ? 'light' : 'dark';
      root.setAttribute('data-theme', next);
      try { localStorage.setItem('theme', next); } catch (e) {}
    });
  }
  /* Non-Mac visitors get one quiet line above the download button. No redirect,
     and the button keeps working, because people download on one machine for
     another. */
  var note = document.getElementById('platform-note');
  if (note) {
    try {
      if (!/Mac/.test(navigator.platform || '')) note.hidden = false;
    } catch (e) {}
  }
})();
