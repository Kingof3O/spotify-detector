/**
 * Stream Manager Setup Controller & Services
 * Modern 3-Column Equal-Height OOP Architecture with Direct Tab Panels & Live Twitch Simulator
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
    this.clearObsPassword = false;
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
      obs: {
        enabled: dom.obsEnabled.checked,
        host: dom.obsHost.value.trim(),
        port: Number(dom.obsPort.value),
        reconnect_min_ms: Number(dom.obsReconnectMin.value),
        reconnect_max_ms: Number(dom.obsReconnectMax.value),
        manual_scene_policy: dom.obsManualPolicy.value
      },
      league: {
        enabled: dom.leagueEnabled.checked,
        game_scene: dom.leagueGameScene.value.trim(),
        client_scene: dom.leagueClientScene.value.trim(),
        idle_scene: dom.leagueIdleScene.value.trim(),
        transition_grace_ms: Number(dom.leagueGrace.value),
        require_foreground: dom.leagueForeground.checked,
        game_process_names: SettingsStore.parseAliases(dom.leagueGameProcesses.value),
        client_process_names: SettingsStore.parseAliases(dom.leagueClientProcesses.value),
        client_window_classes: SettingsStore.parseAliases(dom.leagueClientClasses.value),
        client_window_title_patterns: SettingsStore.parseAliases(dom.leagueClientTitles.value)
      },
      obs_password: this.clearObsPassword ? "" : dom.obsPassword.value,
      clear_obs_password: this.clearObsPassword,
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
 * Main Controller orchestrating DOM interactions, UI feedback, and live chat simulation.
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

      // OBS + League automation
      obsEnabled: document.getElementById("obs-enabled"),
      obsHost: document.getElementById("obs-host"),
      obsPort: document.getElementById("obs-port"),
      obsPassword: document.getElementById("obs-password"),
      obsPasswordStatus: document.getElementById("obs-password-status"),
      obsClearPassword: document.getElementById("obs-clear-password"),
      obsManualPolicy: document.getElementById("obs-manual-policy"),
      obsReconnectMin: document.getElementById("obs-reconnect-min"),
      obsReconnectMax: document.getElementById("obs-reconnect-max"),
      obsStatus: document.getElementById("obs-status"),
      leagueEnabled: document.getElementById("league-enabled"),
      leagueGameScene: document.getElementById("league-game-scene"),
      leagueClientScene: document.getElementById("league-client-scene"),
      leagueIdleScene: document.getElementById("league-idle-scene"),
      leagueGrace: document.getElementById("league-grace"),
      leagueForeground: document.getElementById("league-foreground"),
      leagueGameProcesses: document.getElementById("league-game-processes"),
      leagueClientProcesses: document.getElementById("league-client-processes"),
      leagueClientClasses: document.getElementById("league-client-classes"),
      leagueClientTitles: document.getElementById("league-client-titles"),
      leagueStatus: document.getElementById("league-status"),
      leagueRuntimeStatus: document.getElementById("league-runtime-status"),

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

    inputs.push(
      this.dom.obsEnabled, this.dom.obsHost, this.dom.obsPort, this.dom.obsPassword,
      this.dom.obsManualPolicy, this.dom.obsReconnectMin, this.dom.obsReconnectMax,
      this.dom.leagueEnabled, this.dom.leagueGameScene,
      this.dom.leagueClientScene, this.dom.leagueIdleScene, this.dom.leagueGrace,
      this.dom.leagueForeground, this.dom.leagueGameProcesses, this.dom.leagueClientProcesses,
      this.dom.leagueClientClasses, this.dom.leagueClientTitles
    );

    inputs.forEach((el) => {
      if (!el) return;
      el.addEventListener("input", () => this.store.setDirty(true));
      el.addEventListener("change", () => this.store.setDirty(true));
    });

    this.dom.obsPassword.addEventListener("input", () => {
      this.store.clearObsPassword = false;
    });
    this.dom.obsClearPassword.addEventListener("click", () => {
      this.store.clearObsPassword = true;
      this.dom.obsPassword.value = "";
      this.store.setDirty(true);
      this.showMessage("The saved OBS password will be cleared when you save.");
    });

    // Message inputs preview listeners
    const templateFields = [
      { el: this.dom.msgNowPlaying, trigger: "!song" },
      { el: this.dom.msgPaused, trigger: "!song (paused)" },
      { el: this.dom.msgNothingPlaying, trigger: "!song (idle)" },
      { el: this.dom.msgQueued, trigger: "!sr vampire" },
      { el: this.dom.msgUsage, trigger: "!sr" },
      { el: this.dom.msgPermissionDenied, trigger: "!sr (restricted role)" },
      { el: this.dom.msgCooldown, trigger: "!song (cooldown)" },
      { el: this.dom.msgRequestError, trigger: "!sr error" },
      { el: this.dom.msgNoMatch, trigger: "!sr unknown_song" },
      { el: this.dom.msgNoDevice, trigger: "!sr (no device)" },
      { el: this.dom.msgSpotifyNotConnected, trigger: "!sr (not connected)" },
      { el: this.dom.msgSpotifyAuthExpired, trigger: "!sr (auth expired)" },
      { el: this.dom.msgSpotifyDenied, trigger: "!sr (denied)" },
      { el: this.dom.msgRateLimited, trigger: "!song (rate limited)" },
      { el: this.dom.msgQuotaExceeded, trigger: "!sr (quota exceeded)" }
    ];

    templateFields.forEach(({ el, trigger }) => {
      if (!el) return;
      el.addEventListener("focus", () => this.updateLivePreview(el.value, trigger));
      el.addEventListener("input", () => this.updateLivePreview(el.value, trigger));
    });

    // Category Tabs (No "All Events")
    document.querySelectorAll(".cat-pill").forEach((pill) => {
      pill.addEventListener("click", () => {
        document.querySelectorAll(".cat-pill").forEach((p) => p.classList.remove("active"));
        document.querySelectorAll(".template-tab-panel").forEach((panel) => panel.classList.remove("active"));
        pill.classList.add("active");
        const targetTabId = pill.getAttribute("data-tab");
        const targetPanel = document.getElementById(targetTabId);
        if (targetPanel) {
          targetPanel.classList.add("active");
          const firstInput = targetPanel.querySelector("input");
          const firstField = targetPanel.querySelector(".field");
          const trigger = firstField ? firstField.getAttribute("data-trigger") || "!song" : "!song";
          if (firstInput) {
            this.updateLivePreview(firstInput.value, trigger);
          }
        }
      });
    });

    // Click-to-insert placeholder tag pills
    document.querySelectorAll(".tag-pill").forEach((pill) => {
      pill.addEventListener("click", () => {
        const token = pill.getAttribute("data-insert");
        const parentField = pill.closest(".field");
        const input = parentField ? parentField.querySelector("input") : null;
        const trigger = parentField ? parentField.getAttribute("data-trigger") || "!song" : "!song";
        if (input && token) {
          const start = input.selectionStart ?? input.value.length;
          const end = input.selectionEnd ?? input.value.length;
          const text = input.value;
          input.value = text.substring(0, start) + token + text.substring(end);
          input.selectionStart = input.selectionEnd = start + token.length;
          input.focus();
          this.store.setDirty(true);
          this.updateLivePreview(input.value, trigger);
        }
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

  updateLivePreview(rawText = "", trigger = "!song") {
    this.dom.previewBotName.textContent = `${this.activeBotName}:`;
    this.dom.simUserCmd.textContent = trigger;
    this.dom.simTriggerLabel.textContent = `Trigger: ${trigger}`;

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
    this.store.clearObsPassword = false;
    this.dom.twitchClientId.value = settings.twitch_client_id || "";
    this.dom.twitchChannel.value = settings.twitch_channel || "";
    this.dom.spotifyClientId.value = settings.spotify_client_id || "";

    const obs = settings.obs || {};
    const league = settings.league || {};
    this.dom.obsEnabled.checked = Boolean(obs.enabled);
    this.dom.obsHost.value = obs.host || "127.0.0.1";
    this.dom.obsPort.value = obs.port ?? 4455;
    this.dom.obsReconnectMin.value = obs.reconnect_min_ms ?? 500;
    this.dom.obsReconnectMax.value = obs.reconnect_max_ms ?? 10000;
    this.dom.obsPassword.value = "";
    this.dom.obsManualPolicy.value = obs.manual_scene_policy || "respect";
    this.dom.leagueEnabled.checked = Boolean(league.enabled);
    this.dom.leagueGameScene.value = league.game_scene || "League Game";
    this.dom.leagueClientScene.value = league.client_scene || "League Client";
    this.dom.leagueIdleScene.value = league.idle_scene || "Default";
    this.dom.leagueGrace.value = league.transition_grace_ms ?? 2000;
    this.dom.leagueForeground.checked = Boolean(league.require_foreground);
    this.dom.leagueGameProcesses.value = (league.game_process_names || ["League of Legends.exe"]).join(", ");
    this.dom.leagueClientProcesses.value = (league.client_process_names || ["LeagueClient.exe", "LeagueClientUx.exe"]).join(", ");
    this.dom.leagueClientClasses.value = (league.client_window_classes || []).join(", ");
    this.dom.leagueClientTitles.value = (league.client_window_title_patterns || []).join(", ");

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

    this.updateLivePreview(this.dom.msgNowPlaying.value, "!song");
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

    const automation = data.automation || {};
    const obsConnected = Boolean(automation.obs_connected);
    const obsEnabled = Boolean(automation.obs_enabled);
    this.setStatusBadge(
      this.dom.obsStatus,
      !obsEnabled ? "Disabled" : (obsConnected ? "Connected" : (automation.obs_status || "Connecting")),
      obsEnabled && !obsConnected && String(automation.obs_status || "").startsWith("error")
    );
    this.dom.obsPasswordStatus.textContent = data.obs_password_set
      ? "A password is saved securely on this computer"
      : "No password saved";

    const leagueEnabled = Boolean(data.settings?.league?.enabled);
    this.setStatusBadge(
      this.dom.leagueStatus,
      !leagueEnabled ? "Disabled" : (automation.league_state || "Starting"),
      false
    );
    if (automation.league_state) {
      const parts = [`State: ${automation.league_state}`];
      if (automation.league_pending_transition_ms != null) {
        parts.push(`transition in ${automation.league_pending_transition_ms}ms`);
      }
      if (automation.league_last_signal) parts.push(`last signal: ${automation.league_last_signal}`);
      if (automation.obs_current_scene) parts.push(`OBS: ${automation.obs_current_scene}`);
      this.dom.leagueRuntimeStatus.textContent = parts.join(" · ");
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
