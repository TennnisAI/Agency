/* The only JavaScript on the site beyond the inline pre-paint theme snippet:
   the theme toggle, the hero video's pause button, and the download page's
   one-line notice for visitors on neither macOS nor Linux. */
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
  /* The hero video loops silently, so it needs a way to stop (WCAG 2.2.2
     wants one for anything that moves for more than five seconds). It does
     not start at all for a visitor who asked for reduced motion, and it
     pauses while scrolled out of view rather than decoding frames nobody
     sees. Without JavaScript the autoplay attribute still plays it. */
  var video = document.getElementById('hero-video');
  var toggle = document.getElementById('hero-toggle');
  if (video && toggle) {
    var userPaused = false;
    var label = function () { toggle.textContent = video.paused ? 'Play' : 'Pause'; };
    var play = function () { var p = video.play(); if (p && p.catch) p.catch(function () {}); };
    try {
      if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
        userPaused = true;
        video.removeAttribute('autoplay');
        video.pause();
      }
    } catch (e) {}
    toggle.hidden = false;
    toggle.addEventListener('click', function () {
      userPaused = !video.paused;
      if (userPaused) video.pause(); else play();
    });
    video.addEventListener('play', label);
    video.addEventListener('pause', label);
    label();
    if ('IntersectionObserver' in window) {
      new IntersectionObserver(function (entries) {
        var visible = entries[0].isIntersecting;
        if (!visible) video.pause();
        else if (!userPaused) play();
      }).observe(video);
    }
  }
  /* Visitors on neither platform get one quiet line above the download button.
     No redirect, and every button keeps working, because people download on one
     machine for another. */
  var note = document.getElementById('platform-note');
  if (note) {
    try {
      if (!/Mac|Linux/.test(navigator.platform || '')) note.hidden = false;
    } catch (e) {}
  }
})();
