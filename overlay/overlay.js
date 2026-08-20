(function () {
  "use strict";

  const card = document.querySelector("[data-overlay-card]");
  const artworkShell = document.querySelector("[data-artwork-shell]");
  const artwork = document.querySelector("[data-artwork]");
  const title = document.querySelector("[data-title]");
  const titleTrack = document.querySelector("[data-title-track]");
  const titleText = document.querySelector("[data-title-text]");
  const titleDuplicate = document.querySelector("[data-title-duplicate]");
  const artist = document.querySelector("[data-artist]");
  const source = document.querySelector("[data-source]");
  const paletteCanvas = document.createElement("canvas");
  const paletteSize = 48;
  paletteCanvas.width = paletteSize;
  paletteCanvas.height = paletteSize;
  const paletteContext = paletteCanvas.getContext("2d", { willReadFrequently: true });

  const reconnectDelays = [250, 500, 1000, 2000, 3000, 5000];
  let socket = null;
  let reconnectAttempt = 0;
  let reconnectTimer = 0;
  let staleTimer = 0;
  let currentTrackKey = "";
  let pendingState = null;
  let transitionTimer = 0;
  let titleMeasureFrame = 0;

  if (typeof ResizeObserver === "function") {
    const titleObserver = new ResizeObserver(queueTitleMeasurement);
    titleObserver.observe(title);
  }
  window.addEventListener("resize", queueTitleMeasurement);
  if (document.fonts && document.fonts.ready) {
    document.fonts.ready.then(queueTitleMeasurement);
  }
  window.addEventListener("load", queueTitleMeasurement, { once: true });

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
      return;
    }

    const nextKey = trackKey(next);
    const trackChanged = Boolean(currentTrackKey) && nextKey !== currentTrackKey;

    if (trackChanged) {
      pendingState = next;
      card.classList.add("is-exiting");
      clearTimeout(transitionTimer);
      transitionTimer = window.setTimeout(function () {
        if (!pendingState) return;
        applyContent(pendingState, true);
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

  function applyContent(next, resetMarquee) {
    currentTrackKey = trackKey(next);

    const nextTitle = displayValue(next.title);
    const nextArtist = displayValue(next.artist);
    const nextSource = displayValue(next.source);
    const titleChanged = resetMarquee || titleText.textContent !== nextTitle;

    if (titleChanged) {
      resetTitleMarquee();
    }
    titleText.textContent = nextTitle;
    titleDuplicate.textContent = nextTitle;
    artist.textContent = nextArtist;
    source.textContent = nextSource;
    card.setAttribute(
      "aria-label",
      nextArtist ? `${nextTitle} — ${nextArtist}` : nextTitle,
    );
    card.classList.toggle("is-playing", next.playing === true);
    card.classList.remove("is-hidden");
    // Timeline and playback events can update the state while the same track
    // is playing. Do not remeasure those updates or the marquee will restart.
    if (titleChanged) {
      queueTitleMeasurement();
    }

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
  }

  function hideOverlay() {
    currentTrackKey = "";
    card.classList.add("is-hidden");
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

    const sourceLuminance = luminance(dominant);
    const targetLuminance = Math.min(84, Math.max(46, sourceLuminance * 0.77));
    const scale = sourceLuminance > 0 ? targetLuminance / sourceLuminance : 1;
    const neutral = [17, 17, 20];
    const surface = dominant.map(function (channel, index) {
      const mixed = channel * scale * 0.94 + neutral[index] * 0.06;
      return Math.min(255, Math.max(0, Math.round(mixed / 4) * 4));
    });

    card.style.backgroundColor = rgb(surface);
  }

  function dominantArtworkColor() {
    if (!paletteContext || !artwork.naturalWidth || !artwork.naturalHeight) return null;

    try {
      paletteContext.clearRect(0, 0, paletteSize, paletteSize);
      paletteContext.drawImage(artwork, 0, 0, paletteSize, paletteSize);
      const pixels = paletteContext.getImageData(0, 0, paletteSize, paletteSize).data;
      const buckets = new Map();
      const fallback = { red: 0, green: 0, blue: 0, count: 0 };

      for (let index = 0; index < pixels.length; index += 4) {
        const alpha = pixels[index + 3];
        if (alpha < 192) continue;

        const red = pixels[index];
        const green = pixels[index + 1];
        const blue = pixels[index + 2];
        const pixelLuminance = luminance([red, green, blue]);

        fallback.red += red;
        fallback.green += green;
        fallback.blue += blue;
        fallback.count += 1;

        if (pixelLuminance < 10 || pixelLuminance > 248) continue;

        const quantizedRed = red >> 5;
        const quantizedGreen = green >> 5;
        const quantizedBlue = blue >> 5;
        const key = (quantizedRed << 6) | (quantizedGreen << 3) | quantizedBlue;
        const bucket = buckets.get(key) || {
          red: 0,
          green: 0,
          blue: 0,
          count: 0,
          quantizedRed,
          quantizedGreen,
          quantizedBlue,
        };
        bucket.red += red;
        bucket.green += green;
        bucket.blue += blue;
        bucket.count += 1;
        buckets.set(key, bucket);
      }

      let selected = null;
      let selectedScore = -1;
      for (const candidate of buckets.values()) {
        let score = 0;
        for (const neighbor of buckets.values()) {
          const distance =
            Math.abs(candidate.quantizedRed - neighbor.quantizedRed) +
            Math.abs(candidate.quantizedGreen - neighbor.quantizedGreen) +
            Math.abs(candidate.quantizedBlue - neighbor.quantizedBlue);
          if (distance === 0) score += neighbor.count;
          else if (distance === 1) score += neighbor.count * 0.35;
          else if (distance === 2) score += neighbor.count * 0.12;
        }

        const candidateColor = [
          candidate.red / candidate.count,
          candidate.green / candidate.count,
          candidate.blue / candidate.count,
        ];
        const candidateLuminance = luminance(candidateColor);
        if (candidateLuminance < 24) score *= 0.35;
        else if (candidateLuminance > 232) score *= 0.55;

        if (
          score > selectedScore ||
          (score === selectedScore && selected && candidate.count > selected.count)
        ) {
          selected = candidate;
          selectedScore = score;
        }
      }

      if (selected) {
        const cluster = { red: 0, green: 0, blue: 0, count: 0 };
        for (const neighbor of buckets.values()) {
          const distance =
            Math.abs(selected.quantizedRed - neighbor.quantizedRed) +
            Math.abs(selected.quantizedGreen - neighbor.quantizedGreen) +
            Math.abs(selected.quantizedBlue - neighbor.quantizedBlue);
          const weight = distance === 0 ? 1 : distance === 1 ? 0.35 : distance === 2 ? 0.12 : 0;
          if (!weight) continue;

          cluster.red += neighbor.red * weight;
          cluster.green += neighbor.green * weight;
          cluster.blue += neighbor.blue * weight;
          cluster.count += neighbor.count * weight;
        }
        selected = cluster;
      } else {
        selected = fallback;
      }

      if (!selected.count) return null;
      return [selected.red, selected.green, selected.blue].map(function (total) {
        return Math.round(total / selected.count);
      });
    } catch (_error) {
      return null;
    }
  }

  function resetArtworkPalette() {
    card.style.removeProperty("background-color");
  }

  function queueTitleMeasurement() {
    window.cancelAnimationFrame(titleMeasureFrame);
    titleMeasureFrame = window.requestAnimationFrame(function () {
      titleMeasureFrame = window.requestAnimationFrame(updateTitleMarquee);
    });
  }

  function resetTitleMarquee() {
    title.classList.remove("is-overflowing", "is-measuring");
    title.style.removeProperty("--title-marquee-distance");
    title.style.removeProperty("--title-marquee-offset");
    title.style.removeProperty("--title-marquee-duration");
    titleTrack.style.removeProperty("transform");
  }

  function updateTitleMarquee() {
    titleMeasureFrame = 0;

    resetTitleMarquee();

    const overflow = titleText.scrollWidth - title.clientWidth;
    if (overflow <= 2) return;

    title.classList.add("is-overflowing", "is-measuring");
    const textWidth = Math.max(titleText.scrollWidth, titleText.getBoundingClientRect().width);
    const gap = parseFloat(window.getComputedStyle(titleDuplicate).marginLeft) || 0;
    const distance = textWidth + gap;
    const scrollDuration = Math.min(32, Math.max(4.5, distance / 48));
    const duration = scrollDuration / 0.76;
    title.style.setProperty("--title-marquee-distance", `${distance}px`);
    title.style.setProperty("--title-marquee-offset", `${-distance}px`);
    title.style.setProperty("--title-marquee-duration", `${duration.toFixed(2)}s`);
    title.classList.remove("is-measuring");
  }

  function luminance(channels) {
    return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
  }

  function rgb(channels) {
    return `rgb(${channels.join(", ")})`;
  }

  function displayValue(value) {
    return typeof value === "string" ? value.trim() : "";
  }

  connect();
})();
