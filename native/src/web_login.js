// Injected into the "Войти через discord.com" window before Discord's own scripts.
// Discord removes window.localStorage after loading, so we keep a reference
// taken early and wait for the token to appear once the user has logged in.
// The token is handed to Nimbus by navigating to a fake host that the app
// intercepts and blocks, so it never goes over the network.
(() => {
  if (window.top !== window || location.hostname !== "discord.com") return;
  const storage = window.localStorage;
  const timer = setInterval(() => {
    let raw;
    try {
      raw = storage.getItem("token");
    } catch {
      return;
    }
    if (!raw) return;
    clearInterval(timer);
    location.href = "https://nimbus.callback/#" + JSON.parse(raw);
  }, 500);
})();
