/**
 * Spotify Overlay Setup Controller & Services
 * Modern OOP Architecture with Reactive State, Live Chat Simulation & Header Save
 */

/**
 * Service handling all API communication and CSRF token propagation.
 */
class SetupApiClient {
  constructor() {
    this.csrfToken = "";
  }

  setCsrfToken(token) {
    this.csrfToken = token || "";
  }

  async request(url, options = {}) {
    const headers = {
      ...(options.body ? { "Content-Type": "application/json" } : {}),
      ...(options.headers || {})
    };

    if (options.method && options.method !== "GET" && this.csrfToken) {
      headers["X-Spotify-Overlay-CSRF"] = this.csrfToken;
    }

    const response = await fetch(url, { ...options, headers });
    const body = await response.json().catch(() => ({}));

    if (!response.ok) {
      throw new Error(body.error || body.message || `Request failed (${response.status})`);
    }

    return body;
  }

  getStatus() {
    return this.request("/api/setup/status");
  }

  saveSettings(payload) {
    return this.request("/api/setup/settings", {
      method: "PUT",
      body: JSON.stringify(payload)
    });
  }

  startTwitchAuth() {
    return this.request("/api/auth/twitch/start", { method: "POST" });
  }

  disconnectTwitch() {
    return this.request("/api/auth/twitch/disconnect", { method: "POST" });
  }

  startSpotifyAuth() {
    return this.request("/api/auth/spotify/start", { method: "POST" });
  }

  disconnectSpotify() {
    return this.request("/api/auth/spotify/disconnect", { method: "POST" });
  }
}

/**
 * Reactive Store tracking form state and dirty modifications.
 */
class SettingsStore {
  constructor(onDirtyChange) {
    this.onDirtyChange = onDirtyChange;
    this.isDirty = false;
  }

  setDirty(dirty) {
    this.isDirty = dirty;
    if (this.onDirtyChange) {
      this.onDirtyChange(dirty);
    }
  }

  static parseAliases(raw) {
    return String(raw || "")
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean);
  }

  toPayload(dom) {
    return {
      twitch_client_id: dom.twitchClientId.value.trim(),
      twitch_channel: dom.twitchChannel.value.trim(),
      spotify_client_id: dom.spotifyClientId.value.trim(),
      chat: {
        enabled: dom.chatEnabled.checked,
        requests_enabled: dom.requestsEnabled.checked,
        current_song_commands: SettingsStore.parseAliases(dom.currentSongCommands.value),
        request_commands: SettingsStore.parseAliases(dom.requestCommands.value),
        request_role: dom.requestRole.value,
        request_user_cooldown_secs: Number(dom.userCooldown.value),
        request_global_cooldown_secs: Number(dom.globalCooldown.value),
        messages: {
          now_playing: dom.msgNowPlaying.value,
          paused: dom.msgPaused.value,
          nothing_playing: dom.msgNothingPlaying.value,
          queued: dom.msgQueued.value,
          usage: dom.msgUsage.value,
          permission_denied: dom.msgPermissionDenied.value,
          cooldown: dom.msgCooldown.value,
          request_error: dom.msgRequestError.value,
          no_match: dom.msgNoMatch.value,
          no_device: dom.msgNoDevice.value,
          spotify_not_connected: dom.msgSpotifyNotConnected.value,
          spotify_auth_expired: dom.msgSpotifyAuthExpired.value,
          spotify_denied: dom.msgSpotifyDenied.value,
          rate_limited: dom.msgRateLimited.value,
          quota_exceeded: dom.msgQuotaExceeded.value
        }
      }
    };
  }
}

/**
 * Main Controller orchestrating DOM interactions, UI feedback, and component lifecycles.
 */
class SetupController {
  constructor() {
    this.api = new SetupApiClient();
    this.store = new SettingsStore((dirty) => this.renderDirtyState(dirty));
    this.pollTimer = null;
    this.activeBotName = "OverlayBot";

    this.dom = {
      message: document.getElementById("message"),
      saveBtn: document.getElementById("save"),
      saveBtnText: document.getElementById("save-btn-text"),
      quickDotTwitch: document.getElementById("quick-dot-twitch"),
      quickDotSpotify: document.getElementById("quick-dot-spotify"),
      redirectUri: document.getElementById("redirect-uri"),
      copyRedirectBtn: document.getElementById("copy-redirect"),

      // Live Chat Preview
      previewBotName: document.getElementById("preview-bot-name"),
      previewBotText: document.getElementById("preview-bot-text"),
      previewTemplateLabel: document.getElementById("preview-template-label"),

      // Twitch
      twitchClientId: document.getElementById("twitch-client-id"),
      twitchChannel: document.getElementById("twitch-channel"),
      twitchStatus: document.getElementById("twitch-status"),
      twitchConnectBtn: document.getElementById("twitch-connect"),
      twitchDisconnectBtn: document.getElementById("twitch-disconnect"),
      twitchDevice: document.getElementById("twitch-device"),

      // Spotify
      spotifyClientId: document.getElementById("spotify-client-id"),
      spotifyStatus: document.getElementById("spotify-status"),
      spotifyConnectBtn: document.getElementById("spotify-connect"),
      spotifyDisconnectBtn: document.getElementById("spotify-disconnect"),

      // Chat & Song Requests
      chatEnabled: document.getElementById("chat-enabled"),
      requestsEnabled: document.getElementById("requests-enabled"),
      currentSongCommands: document.getElementById("current-song-commands"),
      requestCommands: document.getElementById("request-commands"),
      requestRole: document.getElementById("request-role"),
      userCooldown: document.getElementById("user-cooldown"),
      globalCooldown: document.getElementById("global-cooldown"),

      // Message Templates
      msgNowPlaying: document.getElementById("message-now-playing"),
      msgPaused: document.getElementById("message-paused"),
      msgNothingPlaying: document.getElementById("message-nothing-playing"),
      msgQueued: document.getElementById("message-queued"),
      msgUsage: document.getElementById("message-usage"),
      msgPermissionDenied: document.getElementById("message-permission-denied"),
      msgCooldown: document.getElementById("message-cooldown"),
      msgRequestError: document.getElementById("message-request-error"),
      msgNoMatch: document.getElementById("message-no-match"),
      msgNoDevice: document.getElementById("message-no-device"),
      msgSpotifyNotConnected: document.getElementById("message-spotify-not-connected"),
      msgSpotifyAuthExpired: document.getElementById("message-spotify-auth-expired"),
      msgSpotifyDenied: document.getElementById("message-spotify-denied"),
      msgRateLimited: document.getElementById("message-rate-limited"),
      msgQuotaExceeded: document.getElementById("message-quota-exceeded")
    };
  }

  async init() {
    this.dom.redirectUri.textContent = `${window.location.origin}/auth/spotify/callback`;
    this.bindEvents();
    await this.load(true);
    this.pollTimer = setInterval(() => this.load(false), 3000);
  }

  bindEvents() {
    // Form change bindings
    const inputs = [
      this.dom.twitchClientId, this.dom.twitchChannel, this.dom.spotifyClientId,
      this.dom.chatEnabled, this.dom.requestsEnabled, this.dom.currentSongCommands,
      this.dom.requestCommands, this.dom.requestRole, this.dom.userCooldown, this.dom.globalCooldown,
      this.dom.msgNowPlaying, this.dom.msgPaused, this.dom.msgNothingPlaying, this.dom.msgQueued,
      this.dom.msgUsage, this.dom.msgPermissionDenied, this.dom.msgCooldown, this.dom.msgRequestError,
      this.dom.msgNoMatch, this.dom.msgNoDevice, this.dom.msgSpotifyNotConnected,
      this.dom.msgSpotifyAuthExpired, this.dom.msgSpotifyDenied, this.dom.msgRateLimited,
      this.dom.msgQuotaExceeded
    ];

    inputs.forEach((el) => {
      if (!el) return;
      el.addEventListener("input", () => this.store.setDirty(true));
      el.addEventListener("change", () => this.store.setDirty(true));
    });

    // Message preview listeners
    const textareas = [
      { el: this.dom.msgNowPlaying, name: "Now Playing" },
      { el: this.dom.msgPaused, name: "Paused Track" },
      { el: this.dom.msgNothingPlaying, name: "Nothing Playing" },
      { el: this.dom.msgQueued, name: "Queued Track" },
      { el: this.dom.msgUsage, name: "Command Usage" },
      { el: this.dom.msgPermissionDenied, name: "Permission Denied" },
      { el: this.dom.msgCooldown, name: "Cooldown" },
      { el: this.dom.msgRequestError, name: "Request Error" },
      { el: this.dom.msgNoMatch, name: "No Match" },
      { el: this.dom.msgNoDevice, name: "No Device" },
      { el: this.dom.msgSpotifyNotConnected, name: "Spotify Not Connected" },
      { el: this.dom.msgSpotifyAuthExpired, name: "Auth Expired" },
      { el: this.dom.msgSpotifyDenied, name: "Request Denied" },
      { el: this.dom.msgRateLimited, name: "Rate Limited" },
      { el: this.dom.msgQuotaExceeded, name: "Quota Exceeded" }
    ];

    textareas.forEach(({ el, name }) => {
      if (!el) return;
      el.addEventListener("focus", () => this.updateLivePreview(el.value, name));
      el.addEventListener("input", () => this.updateLivePreview(el.value, name));
    });

    // Actions
    this.dom.saveBtn.addEventListener("click", () => this.handleSave());
    this.dom.twitchConnectBtn.addEventListener("click", () => this.handleTwitchConnect());
    this.dom.twitchDisconnectBtn.addEventListener("click", () => this.handleTwitchDisconnect());
    this.dom.spotifyConnectBtn.addEventListener("click", () => this.handleSpotifyConnect());
    this.dom.spotifyDisconnectBtn.addEventListener("click", () => this.handleSpotifyDisconnect());

    // Password Visibility Toggles
    document.querySelectorAll("[data-toggle]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const target = document.getElementById(btn.getAttribute("data-toggle"));
        if (!target) return;
        const isPassword = target.type === "password";
        target.type = isPassword ? "text" : "password";
        btn.innerHTML = isPassword
          ? `<i class="fa-solid fa-eye-slash"></i>`
          : `<i class="fa-solid fa-eye"></i>`;
      });
    });

    // 1-Click Copy Redirect URI
    this.dom.copyRedirectBtn.addEventListener("click", async () => {
      const uri = this.dom.redirectUri.textContent;
      if (!uri) return;
      await navigator.clipboard.writeText(uri);
      const originalHTML = this.dom.copyRedirectBtn.innerHTML;
      this.dom.copyRedirectBtn.innerHTML = `<i class="fa-solid fa-check"></i> Copied!`;
      setTimeout(() => { this.dom.copyRedirectBtn.innerHTML = originalHTML; }, 2000);
    });

    // Category Tabs
    document.querySelectorAll(".tab-btn").forEach((btn) => {
      btn.addEventListener("click", () => {
        document.querySelectorAll(".tab-btn").forEach((b) => b.classList.remove("active"));
        document.querySelectorAll(".tab-panel").forEach((p) => p.classList.remove("active"));
        btn.classList.add("active");
        const panel = document.getElementById(btn.getAttribute("data-tab"));
        if (panel) {
          panel.classList.add("active");
          const firstTextarea = panel.querySelector("textarea");
          if (firstTextarea) {
            const label = firstTextarea.previousElementSibling ? firstTextarea.previousElementSibling.textContent : "Message";
            this.updateLivePreview(firstTextarea.value, label);
          }
        }
      });
    });

    // Click-to-insert placeholder tag pills
    document.querySelectorAll(".tag-pill").forEach((pill) => {
      pill.addEventListener("click", () => {
        const token = pill.getAttribute("data-insert");
        const parentField = pill.closest(".field");
        const textarea = parentField ? parentField.querySelector("textarea") : null;
        if (textarea && token) {
          const start = textarea.selectionStart;
          const end = textarea.selectionEnd;
          const text = textarea.value;
          textarea.value = text.substring(0, start) + token + text.substring(end);
          textarea.selectionStart = textarea.selectionEnd = start + token.length;
          textarea.focus();
          this.store.setDirty(true);
          const label = parentField.querySelector("label") ? parentField.querySelector("label").textContent : "Message";
          this.updateLivePreview(textarea.value, label);
        }
      });
    });
  }

  updateLivePreview(rawText, label = "Now Playing") {
    this.dom.previewTemplateLabel.textContent = label;
    this.dom.previewBotName.textContent = `${this.activeBotName}:`;

    if (!rawText || !rawText.trim()) {
      this.dom.previewBotText.innerHTML = `<em>(Message is blank / silenced)</em>`;
      return;
    }

    const rendered = rawText
      .replace(/\{user\}/g, "Alex")
      .replace(/\{track\}/g, "Starboy")
      .replace(/\{title\}/g, "Starboy")
      .replace(/\{artist\}/g, "The Weeknd")
      .replace(/\{seconds\}/g, "25")
      .replace(/\{command\}/g, "!sr <song name>");

    this.dom.previewBotText.textContent = rendered;
  }

  showMessage(text, isError = false) {
    const el = this.dom.message;
    el.textContent = text;
    el.className = text ? (isError ? "error" : "visible") : "";
  }

  renderDirtyState(dirty) {
    if (dirty) {
      this.dom.saveBtn.className = "header-save-btn unsaved";
      this.dom.saveBtnText.textContent = "Save Changes *";
    } else {
      this.dom.saveBtn.className = "header-save-btn";
      this.dom.saveBtnText.textContent = "Save Changes";
    }
  }

  setStatusBadge(element, text, isError = false) {
    element.textContent = text;
    element.className = isError
      ? "status-badge error"
      : (text === "Connected" || text.startsWith("Connected as") ? "status-badge connected" : "status-badge status");
  }

  populateForm(settings) {
    this.dom.twitchClientId.value = settings.twitch_client_id || "";
    this.dom.twitchChannel.value = settings.twitch_channel || "";
    this.dom.spotifyClientId.value = settings.spotify_client_id || "";

    this.dom.chatEnabled.checked = Boolean(settings.chat?.enabled);
    this.dom.requestsEnabled.checked = Boolean(settings.chat?.requests_enabled);
    this.dom.currentSongCommands.value = (settings.chat?.current_song_commands || []).join(", ");
    this.dom.requestCommands.value = (settings.chat?.request_commands || []).join(", ");
    this.dom.requestRole.value = settings.chat?.request_role || "everyone";
    this.dom.userCooldown.value = settings.chat?.request_user_cooldown_secs ?? 30;
    this.dom.globalCooldown.value = settings.chat?.request_global_cooldown_secs ?? 5;

    const msgs = settings.chat?.messages || {};
    this.dom.msgNowPlaying.value = msgs.now_playing || "";
    this.dom.msgPaused.value = msgs.paused || "";
    this.dom.msgNothingPlaying.value = msgs.nothing_playing || "";
    this.dom.msgQueued.value = msgs.queued || "";
    this.dom.msgUsage.value = msgs.usage || "";
    this.dom.msgPermissionDenied.value = msgs.permission_denied || "";
    this.dom.msgCooldown.value = msgs.cooldown || "";
    this.dom.msgRequestError.value = msgs.request_error || "";
    this.dom.msgNoMatch.value = msgs.no_match || "";
    this.dom.msgNoDevice.value = msgs.no_device || "";
    this.dom.msgSpotifyNotConnected.value = msgs.spotify_not_connected || "";
    this.dom.msgSpotifyAuthExpired.value = msgs.spotify_auth_expired || "";
    this.dom.msgSpotifyDenied.value = msgs.spotify_denied || "";
    this.dom.msgRateLimited.value = msgs.rate_limited || "";
    this.dom.msgQuotaExceeded.value = msgs.quota_exceeded || "";

    this.updateLivePreview(this.dom.msgNowPlaying.value, "Now Playing");
  }

  renderStatus(data, overwriteSettings = false) {
    this.api.setCsrfToken(data.csrf_token);

    if (overwriteSettings || !this.store.isDirty) {
      this.populateForm(data.settings);
    }

    const twitchConnected = data.status?.twitch_connected;
    const twitchStatus = data.status?.twitch_status || "Not connected";
    const twitchUser = data.status?.twitch_user;
    if (twitchUser) this.activeBotName = twitchUser;

    this.setStatusBadge(
      this.dom.twitchStatus,
      twitchConnected ? `Connected as ${twitchUser || "bot"}` : twitchStatus,
      !twitchConnected && twitchStatus.startsWith("error")
    );
    this.dom.quickDotTwitch.className = `status-dot ${twitchConnected ? "connected" : (twitchStatus.startsWith("error") ? "error" : "")}`;

    const spotifyConnected = data.status?.spotify_connected;
    const spotifyStatus = data.status?.spotify_status || "Not connected";
    this.setStatusBadge(
      this.dom.spotifyStatus,
      spotifyConnected ? "Connected" : spotifyStatus,
      !spotifyConnected && spotifyStatus.startsWith("error")
    );
    this.dom.quickDotSpotify.className = `status-dot ${spotifyConnected ? "connected" : (spotifyStatus.startsWith("error") ? "error" : "")}`;

    if (data.twitch_device?.state === "pending") {
      this.dom.twitchDevice.textContent = `Authorize at ${data.twitch_device.verification_uri} with code ${data.twitch_device.user_code}.`;
    } else {
      this.dom.twitchDevice.textContent = "";
    }

    if (data.status?.last_error) {
      this.showMessage(`Connection error: ${data.status.last_error}`, true);
    } else if (data.status?.twitch_status === "disabled") {
      this.setStatusBadge(this.dom.twitchStatus, "Commands disabled");
    }
  }

  async load(initial = false) {
    try {
      const data = await this.api.getStatus();
      this.renderStatus(data, initial);
    } catch (error) {
      this.showMessage(error.message, true);
    }
  }

  async handleSave(showMessage = true) {
    try {
      const payload = this.store.toPayload(this.dom);
      const result = await this.api.saveSettings(payload);
      this.store.setDirty(false);
      if (showMessage) {
        this.showMessage("Settings saved successfully.");
      }
      await this.load();
      return result;
    } catch (error) {
      this.showMessage(error.message, true);
    }
  }

  async handleTwitchConnect() {
    try {
      await this.handleSave(false);
      const result = await this.api.startTwitchAuth();
      this.dom.twitchDevice.textContent = `Authorize at ${result.verification_uri} with code ${result.user_code}.`;
      window.open(result.verification_uri, "_blank");
      this.showMessage("Waiting for Twitch authorization…");
    } catch (error) {
      this.showMessage(error.message, true);
    }
  }

  async handleTwitchDisconnect() {
    try {
      await this.api.disconnectTwitch();
      this.showMessage("Twitch disconnected.");
      await this.load();
    } catch (error) {
      this.showMessage(error.message, true);
    }
  }

  async handleSpotifyConnect() {
    try {
      await this.handleSave(false);
      const result = await this.api.startSpotifyAuth();
      window.location.href = result.authorization_url;
    } catch (error) {
      this.showMessage(error.message, true);
    }
  }

  async handleSpotifyDisconnect() {
    try {
      await this.api.disconnectSpotify();
      this.showMessage("Spotify disconnected.");
      await this.load();
    } catch (error) {
      this.showMessage(error.message, true);
    }
  }
}

// Instantiate and initialize on DOM ready
document.addEventListener("DOMContentLoaded", () => {
  const controller = new SetupController();
  controller.init();
});
