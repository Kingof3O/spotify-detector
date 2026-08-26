class AutomationApiClient {
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
    const response = await fetch(url, { ...options, headers, cache: "no-store" });
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
}

class AutomationController {
  constructor() {
    this.api = new AutomationApiClient();
    this.settings = null;
    this.dirty = false;
    this.clearObsPassword = false;
    this.pollTimer = null;
    this.dom = {
      message: document.getElementById("message"),
      save: document.getElementById("save"),
      saveText: document.getElementById("save-btn-text"),
      navStatus: document.getElementById("automation-nav-status"),
      obsStatus: document.getElementById("obs-status"),
      obsEnabled: document.getElementById("obs-enabled"),
      obsHost: document.getElementById("obs-host"),
      obsPort: document.getElementById("obs-port"),
      obsPassword: document.getElementById("obs-password"),
      obsPasswordStatus: document.getElementById("obs-password-status"),
      obsClearPassword: document.getElementById("obs-clear-password"),
      obsManualPolicy: document.getElementById("obs-manual-policy"),
      obsReconnectMin: document.getElementById("obs-reconnect-min"),
      obsReconnectMax: document.getElementById("obs-reconnect-max"),
      leagueStatus: document.getElementById("league-status"),
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
      leagueRuntimeStatus: document.getElementById("league-runtime-status")
    };
  }

  static list(value) {
    return String(value || "")
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean);
  }

  init() {
    this.bindEvents();
    this.load(true);
    this.pollTimer = setInterval(() => this.load(false), 3000);
  }

  bindEvents() {
    const controls = Object.values(this.dom).filter((element) => element instanceof HTMLElement && element.matches("input, select"));
    controls.forEach((element) => {
      element.addEventListener("input", () => this.markDirty());
      element.addEventListener("change", () => this.markDirty());
    });
    this.dom.obsPassword.addEventListener("input", () => {
      this.clearObsPassword = false;
      this.markDirty();
    });
    this.dom.obsClearPassword.addEventListener("click", () => {
      this.clearObsPassword = true;
      this.dom.obsPassword.value = "";
      this.markDirty();
      this.showMessage("The saved OBS password will be cleared when you save.");
    });
    this.dom.save.addEventListener("click", () => this.save());

    document.querySelectorAll("[data-toggle]").forEach((button) => {
      button.addEventListener("click", () => {
        const target = document.getElementById(button.getAttribute("data-toggle"));
        if (!target) return;
        const visible = target.type === "password";
        target.type = visible ? "text" : "password";
        button.innerHTML = visible
          ? `<i class="fa-solid fa-eye-slash"></i>`
          : `<i class="fa-solid fa-eye"></i>`;
      });
    });
  }

  markDirty() {
    this.dirty = true;
    this.dom.save.classList.add("unsaved");
    this.dom.saveText.textContent = "Save Changes *";
  }

  clearDirty() {
    this.dirty = false;
    this.dom.save.classList.remove("unsaved");
    this.dom.saveText.textContent = "Save Changes";
  }

  showMessage(text, isError = false) {
    this.dom.message.textContent = text;
    this.dom.message.className = text ? (isError ? "error" : "visible") : "";
  }

  statusBadge(element, text, error = false) {
    element.textContent = text;
    const normalized = String(text || "").toLowerCase();
    element.className = error
      ? "status-badge error"
      : (normalized.includes("connected") || normalized === "game" || normalized === "client" || normalized === "idle"
        ? "status-badge connected"
        : "status-badge status");
  }

  populate(settings) {
    this.settings = JSON.parse(JSON.stringify(settings || {}));
    const obs = settings.obs || {};
    const league = settings.league || {};
    this.clearObsPassword = false;
    this.dom.obsEnabled.checked = Boolean(obs.enabled);
    this.dom.obsHost.value = obs.host || "127.0.0.1";
    this.dom.obsPort.value = obs.port ?? 4455;
    this.dom.obsPassword.value = "";
    this.dom.obsManualPolicy.value = obs.manual_scene_policy || "respect";
    this.dom.obsReconnectMin.value = obs.reconnect_min_ms ?? 500;
    this.dom.obsReconnectMax.value = obs.reconnect_max_ms ?? 10000;
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
  }

  payload() {
    const payload = JSON.parse(JSON.stringify(this.settings || {}));
    payload.obs = {
      enabled: this.dom.obsEnabled.checked,
      host: this.dom.obsHost.value.trim(),
      port: Number(this.dom.obsPort.value),
      reconnect_min_ms: Number(this.dom.obsReconnectMin.value),
      reconnect_max_ms: Number(this.dom.obsReconnectMax.value),
      manual_scene_policy: this.dom.obsManualPolicy.value
    };
    payload.league = {
      enabled: this.dom.leagueEnabled.checked,
      game_scene: this.dom.leagueGameScene.value.trim(),
      client_scene: this.dom.leagueClientScene.value.trim(),
      idle_scene: this.dom.leagueIdleScene.value.trim(),
      transition_grace_ms: Number(this.dom.leagueGrace.value),
      require_foreground: this.dom.leagueForeground.checked,
      game_process_names: AutomationController.list(this.dom.leagueGameProcesses.value),
      client_process_names: AutomationController.list(this.dom.leagueClientProcesses.value),
      client_window_classes: AutomationController.list(this.dom.leagueClientClasses.value),
      client_window_title_patterns: AutomationController.list(this.dom.leagueClientTitles.value)
    };
    payload.obs_password = this.clearObsPassword ? "" : this.dom.obsPassword.value;
    payload.clear_obs_password = this.clearObsPassword;
    return payload;
  }

  renderStatus(data, overwrite = false) {
    this.api.setCsrfToken(data.csrf_token);
    if (overwrite || !this.dirty) this.populate(data.settings);
    const automation = data.automation || {};
    const obsEnabled = Boolean(data.settings?.obs?.enabled);
    this.statusBadge(
      this.dom.obsStatus,
      !obsEnabled ? "Disabled" : (automation.obs_connected ? "Connected" : (automation.obs_status || "Connecting")),
      obsEnabled && String(automation.obs_status || "").startsWith("error")
    );
    this.dom.obsPasswordStatus.textContent = data.obs_password_set
      ? "A password is saved securely on this computer"
      : "No password saved";

    const leagueEnabled = Boolean(data.settings?.league?.enabled);
    this.statusBadge(this.dom.leagueStatus, !leagueEnabled ? "Disabled" : (automation.league_state || "Starting"));
    const parts = [`State: ${automation.league_state || "unknown"}`];
    if (automation.league_pending_transition_ms != null) parts.push(`transition in ${automation.league_pending_transition_ms}ms`);
    if (automation.league_last_signal) parts.push(`last signal: ${automation.league_last_signal}`);
    if (automation.obs_current_scene) parts.push(`OBS: ${automation.obs_current_scene}`);
    this.dom.leagueRuntimeStatus.textContent = parts.join(" · ");
    this.dom.navStatus.textContent = obsEnabled
      ? `${automation.obs_connected ? "OBS connected" : "OBS offline"} · ${automation.league_state || "unknown"}`
      : "Automation disabled";
  }

  async load(initial = false) {
    try {
      this.renderStatus(await this.api.getStatus(), initial);
    } catch (error) {
      this.showMessage(error.message, true);
    }
  }

  async save() {
    try {
      await this.api.saveSettings(this.payload());
      this.clearDirty();
      this.showMessage("OBS and League settings saved.");
      await this.load(false);
    } catch (error) {
      this.showMessage(error.message, true);
    }
  }
}

document.addEventListener("DOMContentLoaded", () => new AutomationController().init());
