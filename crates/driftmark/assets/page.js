// driftmark's page script. For now it only reloads a page whose content
// changed: a page that shows the working tree reloads when a file changes,
// and the home page reloads when the working tree was counted again. The
// server announces both on /events. The pages lane adds the rest.
"use strict";
(() => {
  const live = document.body && document.body.dataset.live;
  if (!live || !window.EventSource) {
    return;
  }
  const events = new EventSource("/events");
  const reload = () => {
    events.close();
    window.location.reload();
  };
  events.addEventListener(live === "home" ? "counted" : "change", reload);
})();
