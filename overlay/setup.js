/**
 * Stream Manager Setup Controller & Services
 * Modern 3-Column OOP Architecture with Live Twitch Chat Simulation
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

  toPayload(dom, messages) {
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
          now_playing: messages.now_playing || "",
          paused: messages.paused || "",
          nothing_playing: messages.nothing_playing || "",
          queued: messages.queued || "",
          usage: messages.usage || "",
          permission_denied: messages.permission_denied || "",
          cooldown: messages.cooldown || "",
          request_error: messages.request_error || "",
          no_match: messages.no_match || "",
          no_device: messages.no_device || "",
          spotify_not_connected: messages.spotify_not_connected || "",
          spotify_auth_expired: messages.spotify_auth_expired || "",
          spotify_denied: messages.spotify_denied || "",
          rate_limited: messages.rate_limited || "",
          quota_exceeded: messages.quota_exceeded || ""
        }
      }
    };
  }
}

/**
 * Main Controller orchestrating DOM interactions, UI feedback, and single-input template management.
 */
class SetupController {
  constructor() {
    this.api = new SetupApiClient();
    this.store = new SettingsStore((dirty) => this.renderDirtyState(dirty));
    this.pollTimer = null;
    this.activeBotName = "OverlayBot";

    this.currentTemplateKey = "now_playing";
    this.messages = {
      now_playing: "",
      paused: "",
      nothing_playing: "",
      queued: "",
      usage: "",
      permission_denied: "",
      cooldown: "",
      request_error: "",
      no_match: "",
      no_device: "",
      spotify_not_connected: "",
      spotify_auth_expired: "",
      spotify_denied: "",
      rate_limited: "",
      quota_exceeded: ""
    };

    this.dom = {
      message: document.getElementById("message"),
      saveBtn: document.getElementById("save"),
      saveBtnText: document.getElementById("save-btn-text"),
      quickDotTwitch: document.getElementById("quick-dot-twitch"),
      quickDotSpotify: document.getElementById("quick-dot-spotify"),
      redirectUri: document.getElementById("redirect-uri"),
      copyRedirectBtn: document.getElementById("copy-redirect"),

      // Live Chat Preview Simulator
      previewBotName: document.getElementById("preview-bot-name"),
      previewBotText: document.getElementById("preview-bot-text"),
      simUserCmd: document.getElementById("sim-user-cmd"),
      simTriggerLabel: document.getElementById("sim-trigger-label"),

      // Twitch
      twitchClientId: document.getElementById("twitch-client-id"),
      twitchChannel: document.getElementById("twitch-channel"),
      twitchStatus: document.getElementById("twitch-status"),
      twitchSubStatus: document.getElementById("twitch-sub-status"),
      twitchConnectBtn: document.getElementById("twitch-connect"),
      twitchDisconnectBtn: document.getElementById("twitch-disconnect"),
      twitchDevice: document.getElementById("twitch-device"),

      // Spotify
      spotifyClientId: document.getElementById("spotify-client-id"),
      spotifyStatus: document.getElementById("spotify-status"),
      spotifyPermStatus: document.getElementById("spotify-perm-status"),
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

      // Single-Input Message Manager
      templateSelector: document.getElementById("template-selector"),
      activeTemplateInput: document.getElementById("active-template-input"),
      activeTagPills: document.getElementById("active-tag-pills")
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
      this.dom.requestCommands, this.dom.requestRole, this.dom.userCooldown, this.dom.globalCooldown
    ];

    inputs.forEach((el) => {
      if (!el) return;
      el.addEventListener("input", () => this.store.setDirty(true));
      el.addEventListener("change", () => this.store.setDirty(true));
    });

    // Single-input template manager listeners
    this.dom.templateSelector.addEventListener("change", () => {
      this.saveActiveInputToMemory();
      this.currentTemplateKey = this.dom.templateSelector.value;
      this.loadActiveTemplateFromMemory();
    });

    this.dom.activeTemplateInput.addEventListener("input", () => {
      this.messages[this.currentTemplateKey] = this.dom.activeTemplateInput.value;
      this.store.setDirty(true);
      this.updateLivePreview();
    });

    // Category Filter Pills
    document.querySelectorAll(".cat-pill").forEach((pill) => {
      pill.addEventListener("click", () => {
        document.querySelectorAll(".cat-pill").forEach((p) => p.classList.remove("active"));
        pill.classList.add("active");
        const category = pill.getAttribute("data-cat");
        this.filterCategoryOptgroups(category);
      });
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
  }

  filterCategoryOptgroups(category) {
    const optgroups = this.dom.templateSelector.querySelectorAll("optgroup");
    let firstVisibleOption = null;

    optgroups.forEach((group) => {
      const groupCat = group.getAttribute("data-group");
      const match = category === "all" || groupCat === category;
      group.style.display = match ? "" : "none";
      if (match && !firstVisibleOption) {
        firstVisibleOption = group.querySelector("option");
      }
    });

    if (firstVisibleOption) {
      this.saveActiveInputToMemory();
      this.dom.templateSelector.value = firstVisibleOption.value;
      this.currentTemplateKey = firstVisibleOption.value;
      this.loadActiveTemplateFromMemory();
    }
  }

  saveActiveInputToMemory() {
    this.messages[this.currentTemplateKey] = this.dom.activeTemplateInput.value;
  }

  loadActiveTemplateFromMemory() {
    this.dom.activeTemplateInput.value = this.messages[this.currentTemplateKey] || "";
    this.renderTokenPills();
    this.updateLivePreview();
  }

  renderTokenPills() {
    const selectedOption = this.dom.templateSelector.selectedOptions[0];
    const tokensAttr = selectedOption ? selectedOption.getAttribute("data-tokens") : "";
    const container = this.dom.activeTagPills;
    container.innerHTML = "";

    if (!tokensAttr) return;

    const tokens = tokensAttr.split(",").map((t) => t.trim()).filter(Boolean);
    tokens.forEach((token) => {
      const pill = document.createElement("span");
      pill.className = "tag-pill";
      pill.textContent = `+${token}`;
      pill.title = `Insert ${token}`;
      pill.addEventListener("click", () => {
        const input = this.dom.activeTemplateInput;
        const start = input.selectionStart ?? input.value.length;
        const end = input.selectionEnd ?? input.value.length;
        const text = input.value;
        input.value = text.substring(0, start) + token + text.substring(end);
        input.selectionStart = input.selectionEnd = start + token.length;
        input.focus();
        this.messages[this.currentTemplateKey] = input.value;
        this.store.setDirty(true);
        this.updateLivePreview();
      });
      container.appendChild(pill);
    });
  }

  updateLivePreview() {
    const rawText = this.messages[this.currentTemplateKey] || "";
    this.dom.previewBotName.textContent = `${this.activeBotName}:`;

    // Contextual simulated command
    let triggerCmd = "!song";
    if (this.currentTemplateKey === "queued" || this.currentTemplateKey === "no_match" || this.currentTemplateKey === "request_error") {
      triggerCmd = "!sr vampire";
    } else if (this.currentTemplateKey === "usage") {
      triggerCmd = "!sr";
    } else if (this.currentTemplateKey === "cooldown") {
      triggerCmd = "!song (too fast)";
    } else if (this.currentTemplateKey === "permission_denied") {
      triggerCmd = "!sr (viewer role)";
    }

    this.dom.simUserCmd.textContent = triggerCmd;
    this.dom.simTriggerLabel.textContent = `Trigger: ${triggerCmd}`;

    if (!rawText.trim()) {
      this.dom.previewBotText.innerHTML = `<em>(Message is blank / silenced in chat)</em>`;
      return;
    }

    const rendered = rawText
      .replace(/\{user\}/g, "Streamer")
      .replace(/\{track\}/g, "vampire")
      .replace(/\{title\}/g, "vampire")
      .replace(/\{artist\}/g, "Olivia Rodrigo")
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
    Object.keys(this.messages).forEach((key) => {
      this.messages[key] = msgs[key] || "";
    });

    this.loadActiveTemplateFromMemory();
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
    this.dom.twitchSubStatus.textContent = twitchConnected ? "Connected" : "Idle";
    this.dom.twitchSubStatus.style.color = twitchConnected ? "#6ee7b7" : "var(--text-muted)";

    const spotifyConnected = data.status?.spotify_connected;
    const spotifyStatus = data.status?.spotify_status || "Not connected";
    this.setStatusBadge(
      this.dom.spotifyStatus,
      spotifyConnected ? "Connected" : spotifyStatus,
      !spotifyConnected && spotifyStatus.startsWith("error")
    );
    this.dom.quickDotSpotify.className = `status-dot ${spotifyConnected ? "connected" : (spotifyStatus.startsWith("error") ? "error" : "")}`;
    this.dom.spotifyPermStatus.textContent = spotifyConnected ? "Authorized" : "Not Linked";
    this.dom.spotifyPermStatus.style.color = spotifyConnected ? "#6ee7b7" : "var(--text-muted)";

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
      this.saveActiveInputToMemory();
      const payload = this.store.toPayload(this.dom, this.messages);
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
