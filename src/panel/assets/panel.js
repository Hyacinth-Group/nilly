"use strict";

// Claim codes belong in the POST body, never browser history or storage.
const fragment = window.location.hash;
if (fragment) {
  window.history.replaceState(null, "", window.location.pathname + window.location.search);
  const code = new URLSearchParams(fragment.slice(1)).get("token");
  const form = document.getElementById("claim-form");
  if (code && code.length <= 1024 && form instanceof HTMLFormElement) {
    const field = form.elements.namedItem("token");
    if (field instanceof HTMLInputElement) {
      field.value = code;
      form.requestSubmit();
    }
  }
}

function localEndpoint(value) {
  if (!value) return null;
  try {
    const url = new URL(value, window.location.href);
    return url.origin === window.location.origin && !url.username && !url.password ? url : null;
  } catch {
    return null;
  }
}

// Preserve the setup step while switching the browser's language, not BOT_LANG.
const languageSelect = document.querySelector("[data-language-select]");
if (languageSelect) {
  languageSelect.form.addEventListener("submit", (event) => {
    event.preventDefault();
    const url = new URL(window.location.href);
    url.searchParams.set("lang", languageSelect.value);
    window.location.assign(url.pathname + url.search);
  });
}

const main = document.getElementById("main");
const pollUrl = main && localEndpoint(main.dataset.pollUrl);
if (pollUrl) {
  let pending = false;
  let controller;
  let stopped = false;
  const state = document.getElementById("refresh-state");
  const refresh = async () => {
    if (stopped || pending || document.visibilityState !== "visible") return;
    pending = true;
    controller = new AbortController();
    const timeout = window.setTimeout(() => controller.abort(), 7000);
    try {
      const response = await fetch(pollUrl, {
        credentials: "same-origin", cache: "no-store", redirect: "error",
        headers: { Accept: "application/json" }, signal: controller.signal,
      });
      if (!response.ok) throw new Error("snapshot unavailable");
      const snapshot = await response.json();
      if (stopped || document.visibilityState !== "visible") return;
      for (const node of main.querySelectorAll("[data-status-key]")) {
        let value = snapshot;
        for (const part of node.dataset.statusKey.split(".")) {
          value = value && Object.hasOwn(value, part) ? value[part] : undefined;
        }
        if (typeof value === "string" || typeof value === "number") node.textContent = String(value);
      }
      state.hidden = true;
    } catch (error) {
      if (!stopped && document.visibilityState === "visible") {
        state.textContent = main.dataset.refreshError;
        state.hidden = false;
      }
    } finally {
      window.clearTimeout(timeout);
      pending = false;
    }
  };
  const interval = window.setInterval(refresh, 8000);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") refresh();
    else if (controller) controller.abort();
  });
  window.addEventListener("pagehide", () => {
    stopped = true;
    window.clearInterval(interval);
    if (controller) controller.abort();
  }, { once: true });
}

const logs = document.querySelector("[data-log-stream]");
const streamUrl = logs && localEndpoint(logs.dataset.logStream);
if (streamUrl) {
  const capacity = 500;
  const entries = [];
  const lines = document.getElementById("log-lines");
  const state = document.getElementById("log-state");
  const empty = document.getElementById("log-empty");
  const lag = document.getElementById("log-lag");
  const level = document.getElementById("log-level");
  const search = document.getElementById("log-search");
  const pause = document.getElementById("log-pause");
  const follow = document.getElementById("log-follow");
  logs.querySelector(".log-controls").hidden = false;
  state.hidden = false;
  let paused = false;
  let missing = 0;
  let frame = 0;
  const noteMissing = (count) => {
    missing = Math.min(Number.MAX_SAFE_INTEGER, missing + count);
    lag.textContent = logs.dataset.logLag.replace("{count}", String(missing));
    lag.hidden = false;
  };
  const render = () => {
    frame = 0;
    if (paused) return;
    const query = search.value.toLocaleLowerCase();
    const fragment = document.createDocumentFragment();
    let visible = 0;
    for (const entry of entries) {
      if (level.value !== "all" && entry.level !== level.value) continue;
      if (query && !`${entry.timestamp} ${entry.level} ${entry.message}`.toLocaleLowerCase().includes(query)) continue;
      const line = document.createElement("div");
      line.className = "log-line";
      const timestamp = document.createElement("span");
      timestamp.className = "log-timestamp";
      timestamp.textContent = entry.timestamp;
      const severity = document.createElement("span");
      severity.className = `log-severity log-${entry.level}`;
      severity.textContent = entry.level.toUpperCase();
      const message = document.createElement("span");
      message.className = "log-message";
      message.textContent = entry.message;
      line.append(timestamp, severity, message);
      fragment.append(line);
      visible += 1;
    }
    const previousScroll = lines.scrollTop;
    lines.replaceChildren(fragment);
    empty.hidden = visible !== 0;
    lines.scrollTop = follow.checked ? lines.scrollHeight : previousScroll;
  };
  const schedule = () => {
    if (!paused && !frame) frame = window.requestAnimationFrame(render);
  };
  const stream = new EventSource(streamUrl, { withCredentials: true });
  stream.addEventListener("open", () => { state.textContent = logs.dataset.logConnected; });
  stream.addEventListener("error", () => { state.textContent = logs.dataset.logReconnecting; });
  stream.addEventListener("line", (event) => {
    if (event.data.length > 32768) return;
    try {
      const entry = JSON.parse(event.data);
      const normalizedLevel = String(entry.level).toLowerCase();
      if (!['error', 'warn', 'info', 'debug', 'trace'].includes(normalizedLevel) || typeof entry.message !== "string") return;
      if (entries.length === capacity) {
        entries.shift();
        if (paused) noteMissing(1);
      }
      entries.push({
        timestamp: String(entry.timestamp).slice(0, 128),
        level: normalizedLevel,
        message: entry.message.slice(0, 16384),
      });
      schedule();
    } catch {
      // Malformed events are not operational log lines; never interpret them as markup.
    }
  });
  stream.addEventListener("lag", (event) => {
    if (event.data.length > 1024) return;
    try {
      const count = JSON.parse(event.data).dropped;
      if (Number.isSafeInteger(count) && count > 0) noteMissing(count);
    } catch {
      // Ignore malformed control events while retaining the current log buffer.
    }
  });
  level.addEventListener("change", schedule);
  search.addEventListener("input", schedule);
  follow.addEventListener("change", () => { if (follow.checked) lines.scrollTop = lines.scrollHeight; });
  pause.addEventListener("click", () => {
    paused = !paused;
    pause.setAttribute("aria-pressed", String(paused));
    pause.textContent = paused ? logs.dataset.logResume : logs.dataset.logPause;
    if (!paused) schedule();
  });
  window.addEventListener("pagehide", () => {
    stream.close();
    if (frame) window.cancelAnimationFrame(frame);
  }, { once: true });
}
