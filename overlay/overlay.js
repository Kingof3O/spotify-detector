(function () {
  "use strict";

  const card = document.querySelector("[data-overlay-card]");
  const artworkShell = document.querySelector("[data-artwork-shell]");
  const artwork = document.querySelector("[data-artwork]");
  const title = document.querySelector("[data-title]");
  const artist = document.querySelector("[data-artist]");
  const source = document.querySelector("[data-source]");
  const progress = document.querySelector("[data-progress]");
  const paletteCanvas = document.createElement("canvas");
  const paletteSize = 32;
  paletteCanvas.width = paletteSize;
  paletteCanvas.height = paletteSize;
  const paletteContext = paletteCanvas.getContext("2d", { willReadFrequently: true });

  const reconnectDelays = [250, 500, 1000, 2000, 3000, 5000];
  let socket = null;
  let reconnectAttempt = 0;
  let reconnectTimer = 0;
  let staleTimer = 0;
  let animationFrame = 0;
  let currentTrackKey = "";
  let currentState = { available: false };
  let pendingState = null;
  let transitionTimer = 0;
  let timeline = {
    durationMs: 0,
    positionMs: 0,
    playing: false,
    syncedAt: performance.now(),
    correction: null,
  };

  artwork.addEventListener("load", function () {
    artworkShell.classList.add("has-artwork");
    updateArtworkPalette();
  });

  artwork.addEventListener("error", function () {
    artworkShell.classList.remove("has-artwork");
    resetArtworkPalette();
  });

  function connect() {
    clearTimeout(reconnectTimer);

    const protocol = window.location.protocol === "https:" ? "wss" : "ws";
    const host = window.location.host || "127.0.0.1:18923";
    let connection;
    try {
      connection = new WebSocket(`${protocol}://${host}/ws`);
    } catch (_error) {
      scheduleReconnect();
      return;
    }
    socket = connection;

    connection.addEventListener("open", function () {
      if (socket !== connection) return;
      reconnectAttempt = 0;
      clearTimeout(staleTimer);
    });

    connection.addEventListener("message", function (event) {
      if (socket !== connection) return;
      try {
        const message = JSON.parse(event.data);
        if (message && message.type === "state") {
          handleState(message);
        }
      } catch (_error) {
        // The server sends only JSON state messages. Ignore an incomplete frame.
      }
    });

    connection.addEventListener("error", function () {
      connection.close();
    });

    connection.addEventListener("close", function () {
      if (socket !== connection) return;
      scheduleReconnect();
    });
  }

  function scheduleReconnect() {
    clearTimeout(reconnectTimer);
    const delay = reconnectDelays[Math.min(reconnectAttempt, reconnectDelays.length - 1)];
    reconnectAttempt += 1;
    reconnectTimer = window.setTimeout(connect, delay);

    clearTimeout(staleTimer);
    staleTimer = window.setTimeout(function () {
      if (!socket || socket.readyState !== WebSocket.OPEN) {
        hideOverlay();
      }
    }, 12000);
  }

  function handleState(next) {
    if (!next.available) {
      pendingState = null;
      clearTimeout(transitionTimer);
      hideOverlay();
      syncTimeline(next, true);
      return;
    }

    const nextKey = trackKey(next);
    const trackChanged = Boolean(currentTrackKey) && nextKey !== currentTrackKey;

    syncTimeline(next, trackChanged);

    if (trackChanged) {
      pendingState = next;
      card.classList.add("is-exiting");
      clearTimeout(transitionTimer);
      transitionTimer = window.setTimeout(function () {
        if (!pendingState) return;
        applyContent(pendingState);
        pendingState = null;
        card.classList.remove("is-exiting");
        card.classList.add("is-entering");
        window.requestAnimationFrame(function () {
          card.classList.remove("is-entering");
        });
      }, 145);
      return;
    }

    applyContent(next);
  }

  function applyContent(next) {
    currentState = next;
    currentTrackKey = trackKey(next);

    const nextTitle = displayValue(next.title);
    const nextArtist = displayValue(next.artist);
    const nextSource = displayValue(next.source);

    title.textContent = nextTitle;
    artist.textContent = nextArtist;
    source.textContent = nextSource;
    card.setAttribute(
      "aria-label",
      nextArtist ? `${nextTitle} — ${nextArtist}` : nextTitle,
    );
    card.classList.toggle("is-playing", next.playing === true);
    card.classList.remove("is-hidden");

    const artworkUrl = displayValue(next.artwork_url);
    if (artworkUrl) {
      if (artwork.getAttribute("src") !== artworkUrl) {
        artworkShell.classList.remove("has-artwork");
        artwork.setAttribute("src", artworkUrl);
      } else if (artwork.complete && artwork.naturalWidth > 0) {
        artworkShell.classList.add("has-artwork");
        updateArtworkPalette();
      }
    } else {
      artwork.removeAttribute("src");
      artworkShell.classList.remove("has-artwork");
      resetArtworkPalette();
    }

    renderProgress();
  }

  function hideOverlay() {
    currentState = { available: false };
    currentTrackKey = "";
    timeline.playing = false;
    card.classList.add("is-hidden");
  }

  function syncTimeline(next, trackChanged) {
    const now = performance.now();
    const incomingPosition = finiteNumber(next.position_ms, 0);
    const incomingDuration = finiteNumber(next.duration_ms, 0);
    const currentPosition = readPosition(now);
    const correctionIsSmall =
      !trackChanged &&
      timeline.playing &&
      next.playing === true &&
      Math.abs(incomingPosition - currentPosition) <= 750;

    timeline.durationMs = incomingDuration;
    timeline.playing = next.playing === true;
    timeline.syncedAt = now;
    timeline.correction = correctionIsSmall
      ? { from: currentPosition, to: incomingPosition, startedAt: now }
      : null;
    timeline.positionMs = correctionIsSmall ? currentPosition : incomingPosition;
    renderProgress();
  }

  function readPosition(now) {
    if (timeline.correction) {
      const correction = timeline.correction;
      const elapsed = now - correction.startedAt;
      const blend = Math.min(1, Math.max(0, elapsed / 180));
      let position = correction.from + (correction.to - correction.from) * blend;
      if (timeline.playing) position += elapsed;

      if (blend >= 1) {
        timeline.correction = null;
        timeline.positionMs = correction.to + (timeline.playing ? elapsed : 0);
        timeline.syncedAt = now;
      }
      return clamp(position, 0, timeline.durationMs);
    }

    const elapsed = timeline.playing ? Math.max(0, now - timeline.syncedAt) : 0;
    return clamp(timeline.positionMs + elapsed, 0, timeline.durationMs);
  }

  function renderProgress() {
    if (animationFrame) return;
    if (!currentState.available) {
      progress.style.width = "0%";
      return;
    }

    const frame = function () {
      animationFrame = 0;
      if (!currentState.available) {
        progress.style.width = "0%";
        return;
      }

      const now = performance.now();
      const position = readPosition(now);
      const duration = timeline.durationMs;
      const percentage = duration > 0 ? (position / duration) * 100 : 0;
      progress.style.width = `${clamp(percentage, 0, 100)}%`;
      animationFrame = window.requestAnimationFrame(frame);
    };

    animationFrame = window.requestAnimationFrame(frame);
  }

  function trackKey(state) {
    return [state.source, state.title, state.artist, state.album].map(displayValue).join("\u0001");
  }

  function updateArtworkPalette() {
    const dominant = dominantArtworkColor();
    if (!dominant) {
      resetArtworkPalette();
      return;
    }

    const brightestChannel = Math.max(...dominant);
    const scale = brightestChannel > 0 ? Math.min(0.72, 108 / brightestChannel) : 0.72;
    const neutral = [16, 17, 20];
    const surface = dominant.map(function (channel, index) {
      const darkened = channel * scale;
      return Math.round(darkened * 0.9 + neutral[index] * 0.1);
    });
    const border = surface.map(function (channel) {
      return Math.min(255, Math.round(channel * 1.28 + 10));
    });

    card.style.backgroundColor = rgb(surface);
    card.style.borderColor = `rgba(${border.join(", ")}, 0.46)`;
  }

  function dominantArtworkColor() {
    if (!paletteContext || !artwork.naturalWidth || !artwork.naturalHeight) return null;

    try {
      paletteContext.clearRect(0, 0, paletteSize, paletteSize);
      paletteContext.drawImage(artwork, 0, 0, paletteSize, paletteSize);
      const pixels = paletteContext.getImageData(0, 0, paletteSize, paletteSize).data;
      const buckets = new Map();
      const fallback = { red: 0, green: 0, blue: 0, weight: 0 };

      for (let index = 0; index < pixels.length; index += 4) {
        const alpha = pixels[index + 3];
        if (alpha < 160) continue;

        const red = pixels[index];
        const green = pixels[index + 1];
        const blue = pixels[index + 2];
        const maximum = Math.max(red, green, blue);
        const minimum = Math.min(red, green, blue);
        const luminance = red * 0.2126 + green * 0.7152 + blue * 0.0722;

        fallback.red += red;
        fallback.green += green;
        fallback.blue += blue;
        fallback.weight += 1;

        if (luminance < 18 || luminance > 242) continue;

        const key = `${red >> 4}:${green >> 4}:${blue >> 4}`;
        const saturation = maximum - minimum;
        const weight = (1 + saturation / 160) * (luminance < 34 ? 0.35 : 1);
        const bucket = buckets.get(key) || { red: 0, green: 0, blue: 0, weight: 0 };
        bucket.red += red * weight;
        bucket.green += green * weight;
        bucket.blue += blue * weight;
        bucket.weight += weight;
        buckets.set(key, bucket);
      }

      let dominant = null;
      for (const bucket of buckets.values()) {
        if (!dominant || bucket.weight > dominant.weight) dominant = bucket;
      }

      const selected = dominant || fallback;
      if (!selected.weight) return null;
      return [selected.red, selected.green, selected.blue].map(function (total) {
        return Math.round(total / selected.weight);
      });
    } catch (_error) {
      return null;
    }
  }

  function resetArtworkPalette() {
    card.style.removeProperty("background-color");
    card.style.removeProperty("border-color");
  }

  function rgb(channels) {
    return `rgb(${channels.join(", ")})`;
  }

  function displayValue(value) {
    return typeof value === "string" ? value.trim() : "";
  }

  function finiteNumber(value, fallback) {
    return typeof value === "number" && Number.isFinite(value) ? Math.max(0, value) : fallback;
  }

  function clamp(value, minimum, maximum) {
    if (maximum <= 0) return Math.max(0, value);
    return Math.min(maximum, Math.max(minimum, value));
  }

  connect();
})();
