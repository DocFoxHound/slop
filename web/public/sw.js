// Slop's service worker: phone and browser notifications (crates/slop-api/src/push.rs).
//
// A push from Slop carries nothing. This wakes, fetches the newest notification with the user's
// own sign-in, and shows it, so the push service never learns what it was about.

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

self.addEventListener("push", (event) => {
  event.waitUntil(
    (async () => {
      let title = "Slop";
      let options = { body: "You have something new on Slop.", icon: "/logo.svg", badge: "/logo.svg", data: { link: "/notifications" } };
      try {
        const res = await fetch("/v1/me/notifications", { credentials: "include", headers: { Accept: "application/json" } });
        if (res.ok) {
          const inbox = await res.json();
          const n = (inbox.items || []).find((i) => !i.read);
          if (n) {
            title = n.count > 1 ? `${n.title} (${n.count})` : n.title;
            options = { ...options, body: n.body, tag: n.id, renotify: true, data: { link: n.link || "/notifications" } };
          }
        }
      } catch {
        // Offline or signed out: the general notice above.
      }
      await self.registration.showNotification(title, options);
    })(),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const link = new URL(event.notification.data?.link || "/notifications", self.location.origin).href;
  event.waitUntil(
    (async () => {
      const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
      const open = windows.find((w) => new URL(w.url).origin === self.location.origin);
      if (open) {
        await open.focus();
        return open.navigate(link);
      }
      return self.clients.openWindow(link);
    })(),
  );
});
