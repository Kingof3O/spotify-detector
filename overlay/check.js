/**
 * Spotify Overlay Health Check Controller & Renderer
 * Modern OOP Architecture with Encapsulated API & Component Renderers
 */

/**
 * Service handling health check API requests.
 */
class HealthCheckApiClient {
  async fetchLiveCheck() {
    const response = await fetch("/api/health/check?live=1", { cache: "no-store" });
    if (!response.ok) {
      throw new Error(`Health check failed with HTTP ${response.status}`);
    }
    return response.json();
  }
}

/**
 * Utility Renderer for SVG icons, escape helpers, and diagnostic items.
 */
class DiagnosticRenderer {
  static escape(value) {
    return String(value ?? "").replace(/[&<>\"]/g, (char) => ({
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;"
    }[char]));
  }

  static getStatusIcon(status) {
    if (status === "ok") {
      return `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>`;
    } else if (status === "warning") {
      return `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"></path><line x1="12" y1="9" x2="12" y2="13"></line><line x1="12" y1="17" x2="12.01" y2="17"></line></svg>`;
    }
    return `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>`;
  }

  static renderCheckItem(check) {
    const statusClass = DiagnosticRenderer.escape(check.status);
    const actionUrl = check.action && check.action.includes("setup") ? "/setup" : null;
    const nameEscaped = DiagnosticRenderer.escape(check.name);
    const detailEscaped = DiagnosticRenderer.escape(check.detail);
    const actionEscaped = DiagnosticRenderer.escape(check.action);

    return `
      <article class="check-item">
        <div class="check-title-cell">
          <div class="check-icon ${statusClass}">
            ${DiagnosticRenderer.getStatusIcon(check.status)}
          </div>
          <span class="check-name">${nameEscaped}</span>
        </div>
        <div>
          <span class="state-pill ${statusClass}">${statusClass}</span>
        </div>
        <div class="check-detail-cell">
          <div class="check-detail">${detailEscaped}</div>
          ${check.action ? (
            actionUrl
              ? `<a href="${actionUrl}" class="check-action-link"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="5" y1="12" x2="19" y2="12"></line><polyline points="12 5 19 12 12 19"></polyline></svg> Next: ${actionEscaped}</a>`
              : `<span class="check-action-link"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="5" y1="12" x2="19" y2="12"></line><polyline points="12 5 19 12 12 19"></polyline></svg> Next: ${actionEscaped}</span>`
          ) : ""}
        </div>
      </article>
    `;
  }
}

/**
 * Controller orchestrating the diagnostics dashboard.
 */
class HealthCheckController {
  constructor() {
    this.api = new HealthCheckApiClient();

    this.dom = {
      summary: document.getElementById("summary"),
      checksContainer: document.getElementById("checks"),
      updated: document.getElementById("updated"),
      refreshBtn: document.getElementById("refresh"),
      refreshIcon: document.getElementById("refresh-icon"),
      heroPulse: document.getElementById("hero-pulse"),
      heroTitle: document.getElementById("hero-title"),

      // Metrics
      metricDotServer: document.getElementById("metric-dot-server"),
      metricValServer: document.getElementById("metric-val-server"),
      metricDotMedia: document.getElementById("metric-dot-media"),
      metricValMedia: document.getElementById("metric-val-media"),
      metricDotTwitch: document.getElementById("metric-dot-twitch"),
      metricValTwitch: document.getElementById("metric-val-twitch"),
      metricDotSpotify: document.getElementById("metric-dot-spotify"),
      metricValSpotify: document.getElementById("metric-val-spotify")
    };
  }

  init() {
    this.dom.refreshBtn.addEventListener("click", () => this.refresh());
    this.refresh();
  }

  async refresh() {
    this.dom.refreshIcon.classList.add("spin");
    this.dom.summary.textContent = "Running live diagnostics…";

    try {
      const report = await this.api.fetchLiveCheck();
      this.renderReport(report);
    } catch (error) {
      this.renderError(error);
    } finally {
      this.dom.refreshIcon.classList.remove("spin");
    }
  }

  renderReport(report) {
    const overall = report.overall || "ok";
    this.dom.heroPulse.className = `status-pulse-ring ${overall}`;
    this.dom.heroTitle.textContent =
      overall === "ok"
        ? "All Systems Operational"
        : overall === "warning"
        ? "Needs Attention"
        : "System Issues Detected";

    this.dom.summary.textContent = report.summary;

    // Render diagnostic checklist items
    this.dom.checksContainer.innerHTML = (report.checks || [])
      .map((check) => DiagnosticRenderer.renderCheckItem(check))
      .join("");

    // Update top metric indicators
    this.updateMetrics(report);

    this.dom.updated.textContent = `Last checked at ${new Date(report.checked_at * 1000).toLocaleTimeString()}`;
  }

  updateMetrics(report) {
    const checks = report.checks || [];
    const findCheck = (term) => checks.find((c) => c.name.toLowerCase().includes(term));

    const serverCheck = findCheck("server");
    const mediaCheck = findCheck("media");
    const twitchCheck = findCheck("twitch authorization") || findCheck("twitch");
    const spotifyCheck = findCheck("spotify playback") || findCheck("spotify authorization") || findCheck("spotify");

    if (serverCheck) {
      this.dom.metricDotServer.className = `metric-dot ${serverCheck.status}`;
      this.dom.metricValServer.textContent = `v${report.version || "0.1.0"}`;
    }
    if (mediaCheck) {
      this.dom.metricDotMedia.className = `metric-dot ${mediaCheck.status}`;
      this.dom.metricValMedia.textContent = mediaCheck.status === "ok" ? "Session Active" : "No Track";
    }
    if (twitchCheck) {
      this.dom.metricDotTwitch.className = `metric-dot ${twitchCheck.status}`;
      this.dom.metricValTwitch.textContent = twitchCheck.status === "ok" ? "Connected" : "Disconnected";
    }
    if (spotifyCheck) {
      this.dom.metricDotSpotify.className = `metric-dot ${spotifyCheck.status}`;
      this.dom.metricValSpotify.textContent = spotifyCheck.status === "ok" ? "Authorized" : "Disconnected";
    }
  }

  renderError(error) {
    this.dom.heroPulse.className = "status-pulse-ring error";
    this.dom.heroTitle.textContent = "Diagnostic Check Failed";
    this.dom.summary.textContent = `Could not communicate with local server: ${error.message}`;
    this.dom.checksContainer.innerHTML = "";
  }
}

// Instantiate and initialize on DOM ready
document.addEventListener("DOMContentLoaded", () => {
  const controller = new HealthCheckController();
  controller.init();
});
