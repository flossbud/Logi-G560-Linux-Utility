// G560 Linux Utility GUI — full behaviour for Lighting, Setup & Service,
// Diagnostics, and About pages. The service is the source of truth;
// every user action dispatches a typed Tauri command and the UI paints
// from the returned or broadcast ServiceSnapshot.

const invoke = window.__TAURI__?.core?.invoke;
const listen = window.__TAURI__?.event?.listen;

// Report frontend problems into the Rust tracing subscriber so bug reports
// have real evidence without asking users to open WebView devtools.
function report(msg) {
  console.warn("[frontend]", msg);
  if (invoke) {
    invoke("frontend_log", { message: String(msg) }).catch(() => {});
  }
}

if (!invoke || !listen) {
  report(
    `Tauri globals missing: invoke=${!!invoke} listen=${!!listen} — check tauri.conf.json withGlobalTauri`,
  );
}

window.addEventListener("error", (e) => {
  report(`window.error: ${e.message} @ ${e.filename}:${e.lineno}`);
});
window.addEventListener("unhandledrejection", (e) => {
  report(`unhandled: ${e.reason}`);
});

const ZONES = ["left-rear", "left-front", "right-front", "right-rear"];
const GROUPS = {
  all: ZONES,
  fronts: ["left-front", "right-front"],
  rears: ["left-rear", "right-rear"],
  left: ["left-rear", "left-front"],
  right: ["right-rear", "right-front"],
};

const state = {
  snapshot: null,
  selected: new Set(ZONES),
  pickerHex: "#14C8F4",
  brightness: 100,
  connection: { kind: "connecting" },
  activePage: "lighting",
  udev: null,
  serviceUnit: null,
  version: null,
};

const panel = document.querySelector("[data-interactive-lighting]");
const q = (sel, root = panel) => root.querySelector(sel);
const qa = (sel, root = panel) => root.querySelectorAll(sel);

const firstRunOverlay = document.querySelector("[data-first-run]");
const firstRunStart = document.querySelector("[data-first-run-start]");
const firstRunSkip = document.querySelector("[data-first-run-skip]");

const els = {
  serviceDot: q("[data-service-dot]"),
  serviceLabel: q("[data-service-label]"),
  deviceDot: q("[data-device-dot]"),
  deviceLabel: q("[data-device-label]"),
  masterSwitch: q("[data-master-switch]"),
  masterLabel: q("[data-master-label]"),

  rail: qa("[data-page]"),
  pages: qa("[data-page-body]"),

  // Lighting
  modeTabs: qa("[data-mode-tab]"),
  modeBadge: q("[data-mode-badge]"),
  liveLabel: q("[data-live-label]"),
  controls: q(".controls"),
  banner: q("[data-banner]"),
  bannerText: q("[data-banner-text]"),
  zones: qa("[data-zone]"),
  groups: qa("[data-group]"),
  colorInput: q("[data-color-input]"),
  paletteButtons: qa("[data-palette-color]"),
  hexOutput: q("[data-hex-output]"),
  rgbOutput: q("[data-rgb-output]"),
  selectionCount: q("[data-selection-count]"),
  selectionCopy: q("[data-selection-copy]"),
  brightnessInput: q("[data-brightness-input]"),
  brightnessValue: q("[data-brightness-value]"),
  modePanels: {
    manual: q('[data-mode-panel="manual"]'),
    "content-aware": q('[data-mode-panel="content-aware"]'),
  },
  lightZones: {
    "left-rear": qa('[data-light-zone="left-rear"]'),
    "left-front": qa('[data-light-zone="left-front"]'),
    "right-front": qa('[data-light-zone="right-front"]'),
    "right-rear": qa('[data-light-zone="right-rear"]'),
  },

  // Content-Aware panel
  caBackend: q("[data-ca-backend]"),
  caCapture: q("[data-ca-capture]"),
  caRate: q("[data-ca-rate]"),
  caStalls: q("[data-ca-stalls]"),
  chooseDisplay: q("[data-choose-display]"),
  restartCapture: q("[data-restart-capture]"),

  // Setup page
  udevRule: q("[data-udev-rule]"),
  udevDevice: q("[data-udev-device]"),
  udevWritable: q("[data-udev-writable]"),
  installUdev: q("[data-install-udev]"),
  refreshUdev: q("[data-refresh-udev]"),
  udevOutcome: q("[data-udev-outcome]"),

  serviceInstalled: q("[data-service-installed]"),
  serviceEnabled: q("[data-service-enabled]"),
  serviceActive: q("[data-service-active]"),
  servicePath: q("[data-service-path]"),
  installService: q("[data-install-service]"),
  serviceStart: q("[data-service-start]"),
  serviceStop: q("[data-service-stop]"),
  serviceRestart: q("[data-service-restart]"),
  refreshService: q("[data-refresh-service]"),
  uninstallService: q("[data-uninstall-service]"),
  serviceOutcome: q("[data-service-outcome]"),

  gamingInstalled: q("[data-gaming-installed]"),
  gamingEnabled: q("[data-gaming-enabled]"),
  gamingActive: q("[data-gaming-active]"),
  installGaming: q("[data-install-gaming]"),
  uninstallGaming: q("[data-uninstall-gaming]"),
  refreshGaming: q("[data-refresh-gaming]"),
  gamingOutcome: q("[data-gaming-outcome]"),

  launcherBanner: q("[data-launcher-banner]"),
  launcherOldPath: q("[data-launcher-old-path]"),
  relinkLauncher: q("[data-relink-launcher]"),

  // Diagnostics
  diagService: q("[data-diag-service]"),
  diagWriter: q("[data-diag-writer]"),
  diagCapture: q("[data-diag-capture]"),
  diagMode: q("[data-diag-mode]"),
  diagBackend: q("[data-diag-backend]"),
  diagFrames: q("[data-diag-frames]"),
  diagNewest: q("[data-diag-newest]"),
  diagStalls: q("[data-diag-stalls]"),
  diagUsbfail: q("[data-diag-usbfail]"),
  diagUsbrec: q("[data-diag-usbrec]"),
  diagRate: q("[data-diag-rate]"),
  diagRevision: q("[data-diag-revision]"),
  copyDiag: q("[data-copy-diagnostics]"),
  copyFeedback: q("[data-copy-feedback]"),

  // About
  aboutGui: q("[data-about-gui]"),
  aboutApi: q("[data-about-api]"),
  aboutBus: q("[data-about-bus]"),
  aboutTarget: q("[data-about-target]"),
  aboutDistro: q("[data-about-distro]"),
  aboutSession: q("[data-about-session]"),
  aboutDesktop: q("[data-about-desktop]"),
  aboutKernel: q("[data-about-kernel]"),
  copyVersion: q("[data-copy-version]"),
  versionFeedback: q("[data-version-feedback]"),
};

// -- utilities --------------------------------------------------------

function normalizeHex(value) {
  const upper = String(value).trim().toUpperCase();
  const match = /^#?([0-9A-F]{6})$/.exec(upper);
  if (!match) throw new Error(`invalid hex color: ${value}`);
  return `#${match[1]}`;
}
function hexToRgb(hex) {
  const s = normalizeHex(hex).slice(1);
  return {
    r: parseInt(s.slice(0, 2), 16),
    g: parseInt(s.slice(2, 4), 16),
    b: parseInt(s.slice(4, 6), 16),
  };
}
function rgbToHex({ red, green, blue }) {
  return (
    "#" +
    [red, green, blue]
      .map((v) => v.toString(16).padStart(2, "0"))
      .join("")
      .toUpperCase()
  );
}
function confirmedColorForZone(zone) {
  if (!state.snapshot) return "#14C8F4";
  const entry = state.snapshot.confirmed_colors.find((z) => z.zone === zone);
  return entry ? rgbToHex(entry.color) : "#14C8F4";
}
function manualSettingForZone(zone) {
  if (!state.snapshot) return null;
  return state.snapshot.manual_zones.find((z) => z.zone === zone) || null;
}
function mixedSelection(getter) {
  const values = [...state.selected].map(getter);
  if (values.length === 0) return null;
  const first = values[0];
  const mixed = values.some((v) => JSON.stringify(v) !== JSON.stringify(first));
  return mixed ? "mixed" : first;
}
function firstSelected() {
  return state.selected.values().next().value || null;
}
function zoneLabel(zone) {
  return zone
    .split("-")
    .map((w) => w[0].toUpperCase() + w.slice(1))
    .join(" ");
}
function backendLabel(backend) {
  if (backend === "desktop-portal") return "Desktop Portal";
  if (backend === "gamescope") return "Gamescope";
  if (!backend) return "—";
  return backend;
}
function healthClass(v) {
  if (v === "ready") return "ok";
  if (v === "recovering" || v === "starting") return "warn";
  if (v === "failed" || v === "unavailable") return "error";
  return "";
}

// -- page navigation --------------------------------------------------

function switchPage(name) {
  state.activePage = name;
  els.rail.forEach((btn) => {
    const active = btn.dataset.page === name;
    btn.classList.toggle("active", active);
    if (active) btn.setAttribute("aria-current", "page");
    else btn.removeAttribute("aria-current");
  });
  els.pages.forEach((section) => {
    const isActive = section.dataset.pageBody === name;
    section.classList.toggle("active", isActive);
    section.hidden = !isActive;
  });
  if (name === "setup") refreshSetup();
  if (name === "diagnostics") renderDiagnostics();
  if (name === "about") loadVersion();
}

function bindRail() {
  els.rail.forEach((btn) => {
    btn.addEventListener("click", () => switchPage(btn.dataset.page));
  });
}

// -- render (Lighting + top status) -----------------------------------

function render() {
  renderConnection();
  renderMaster();
  renderMode();
  renderZones();
  renderGroups();
  renderColor();
  renderBrightness();
  renderStage();
  renderSelectionCopy();
  renderContentAware();
  renderFirstRun();
  if (state.activePage === "diagnostics") renderDiagnostics();
}

function renderFirstRun() {
  const snap = state.snapshot;
  const needed = snap ? !snap.setup_complete : false;
  firstRunOverlay.hidden = !needed;
}

async function markSetupComplete() {
  try {
    await invoke("mark_setup_complete");
  } catch (err) {
    console.warn("mark_setup_complete failed", err);
  }
}

function renderConnection() {
  const conn = state.connection || { kind: "disconnected" };
  const dot = els.serviceDot;
  dot.classList.remove("ok", "warn", "error");
  if (conn.kind === "connected") {
    dot.classList.add("ok");
    els.serviceLabel.textContent = "Service running";
  } else if (conn.kind === "connecting") {
    dot.classList.add("warn");
    els.serviceLabel.textContent = "Connecting…";
  } else {
    dot.classList.add("error");
    els.serviceLabel.textContent = "Service unreachable";
  }

  const deviceDot = els.deviceDot;
  deviceDot.classList.remove("ok", "warn", "error");
  const snap = state.snapshot;
  const health = snap ? snap.device_health : "unknown";
  const writer = snap ? snap.writer_health : "unknown";
  if (writer === "ready") {
    deviceDot.classList.add("ok");
    els.deviceLabel.textContent = "G560 connected";
  } else if (writer === "failed" || health === "failed") {
    deviceDot.classList.add("error");
    els.deviceLabel.textContent = "G560 unavailable";
  } else if (health === "recovering" || writer === "recovering") {
    deviceDot.classList.add("warn");
    els.deviceLabel.textContent = "G560 recovering";
  } else {
    els.deviceLabel.textContent = "G560 initialising";
  }

  if (conn.kind === "disconnected") {
    showBanner(`Lighting service unreachable: ${conn.reason || "unknown"}`);
  } else if (snap && snap.writer_health === "failed") {
    showBanner("Last hardware write failed. Preview shows the last confirmed state.");
  } else {
    hideBanner();
  }
}

function renderMaster() {
  const enabled = state.snapshot ? state.snapshot.lights_enabled : true;
  els.masterSwitch.setAttribute("aria-pressed", String(enabled));
  els.masterLabel.textContent = enabled ? "ON" : "OFF";
  els.controls.classList.toggle("locked", !enabled);
}

function renderMode() {
  const mode = state.snapshot ? state.snapshot.mode : "manual";
  els.modeTabs.forEach((tab) => {
    const active = tab.dataset.modeTab === mode;
    tab.classList.toggle("active", active);
    tab.setAttribute("aria-selected", String(active));
  });
  els.modePanels.manual.hidden = mode !== "manual";
  els.modePanels["content-aware"].hidden = mode !== "content-aware";
  els.modeBadge.textContent =
    mode === "content-aware" ? "Content-Aware mode" : "Manual mode";
  els.liveLabel.textContent =
    mode === "content-aware" ? "Live capture" : "Newest value only";
  els.controls.classList.toggle(
    "locked",
    state.snapshot && !state.snapshot.lights_enabled,
  );
}

function renderZones() {
  els.zones.forEach((btn) => {
    const zone = btn.dataset.zone;
    const isSelected = state.selected.has(zone);
    btn.classList.toggle("selected", isSelected);
    btn.setAttribute("aria-pressed", String(isSelected));
    const led = btn.querySelector(".led");
    if (led) {
      const color = confirmedColorForZone(zone);
      led.style.backgroundColor = color;
      led.style.boxShadow = `0 0 10px ${color}`;
    }
  });
}
function renderGroups() {
  els.groups.forEach((btn) => {
    const group = GROUPS[btn.dataset.group];
    const active =
      group.length === state.selected.size &&
      group.every((zone) => state.selected.has(zone));
    btn.classList.toggle("active", active);
  });
}
function renderColor() {
  const setting = mixedSelection((zone) => {
    const s = manualSettingForZone(zone);
    return s ? rgbToHex(s.color) : "#14C8F4";
  });
  if (setting === "mixed") {
    els.colorInput.value = state.pickerHex.toLowerCase();
    els.hexOutput.textContent = "MIXED";
    els.rgbOutput.innerHTML =
      "<span>R</span><b>—</b><span>G</span><b>—</b><span>B</span><b>—</b>";
    return;
  }
  const hex = setting || state.pickerHex;
  state.pickerHex = hex;
  els.colorInput.value = hex.toLowerCase();
  const { r, g, b } = hexToRgb(hex);
  els.hexOutput.textContent = hex.slice(1);
  els.rgbOutput.innerHTML = `<span>R</span><b>${r}</b><span>G</span><b>${g}</b><span>B</span><b>${b}</b>`;
}
function renderBrightness() {
  const value = mixedSelection((zone) => {
    const s = manualSettingForZone(zone);
    return s ? s.brightness : 100;
  });
  if (value === "mixed") {
    els.brightnessValue.textContent = "mixed";
    els.brightnessInput.value = state.brightness;
  } else {
    const v = value == null ? state.brightness : value;
    state.brightness = v;
    els.brightnessInput.value = String(v);
    els.brightnessValue.textContent = `${v}%`;
  }
  els.brightnessInput.style.setProperty("--pct", `${els.brightnessInput.value}%`);
}
function renderStage() {
  ZONES.forEach((zone) => {
    const color = confirmedColorForZone(zone);
    els.lightZones[zone].forEach((el) => {
      el.style.setProperty("--zone-color", color);
      el.dataset.color = color;
    });
  });
}
function renderSelectionCopy() {
  const count = state.selected.size;
  const s = count === 1 ? "" : "s";
  els.selectionCount.textContent = `${count} zone${s} selected`;
  let label;
  if (count === 4) label = "All four zones";
  else if (count === 0) label = "No zones";
  else if (count === 1) label = zoneLabel(firstSelected());
  else label = `${count} zones`;
  els.selectionCopy.textContent = label;
}

function renderContentAware() {
  const snap = state.snapshot;
  els.caBackend.textContent = backendLabel(snap ? snap.capture_backend : null);
  els.caCapture.textContent = snap ? snap.capture_health : "—";
  const rate = snap ? snap.diagnostics.capture_rate_millihertz : 0;
  els.caRate.textContent = rate > 0 ? `${(rate / 1000).toFixed(1)} fps` : "—";
  els.caStalls.textContent = snap ? snap.diagnostics.capture_stalls : 0;
}

function showBanner(text) {
  els.bannerText.textContent = text;
  els.banner.hidden = false;
}
function hideBanner() {
  els.banner.hidden = true;
}

// -- IPC dispatch ------------------------------------------------------

async function pushManualUpdates(zones, color, brightness) {
  if (zones.length === 0) return;
  const updates = zones.map((zone) => ({
    zone,
    color: color != null ? hexToApiColor(color) : currentZoneColor(zone),
    brightness: brightness != null ? brightness : currentZoneBrightness(zone),
  }));
  try {
    const result = await invoke("set_manual_zones", { updates });
    if (result.status === "err") showBanner(`Rejected: ${result.error}`);
  } catch (err) {
    showBanner(`Command failed: ${err}`);
  }
}
function hexToApiColor(hex) {
  const { r, g, b } = hexToRgb(hex);
  return { red: r, green: g, blue: b };
}
function currentZoneColor(zone) {
  const setting = manualSettingForZone(zone);
  if (setting) return setting.color;
  const { r, g, b } = hexToRgb(state.pickerHex);
  return { red: r, green: g, blue: b };
}
function currentZoneBrightness(zone) {
  const setting = manualSettingForZone(zone);
  if (setting) return setting.brightness;
  return state.brightness;
}

// -- Lighting bindings -------------------------------------------------

function bindZones() {
  els.zones.forEach((btn) => {
    btn.addEventListener("click", () => {
      const zone = btn.dataset.zone;
      if (state.selected.has(zone)) state.selected.delete(zone);
      else state.selected.add(zone);
      render();
    });
  });
}
function bindGroups() {
  els.groups.forEach((btn) => {
    btn.addEventListener("click", () => {
      state.selected = new Set(GROUPS[btn.dataset.group]);
      render();
    });
  });
}
function bindColor() {
  els.colorInput.addEventListener("input", (event) => {
    state.pickerHex = normalizeHex(event.target.value);
    pushManualUpdates([...state.selected], state.pickerHex, null);
  });
  els.paletteButtons.forEach((btn) => {
    btn.addEventListener("click", () => {
      const color = normalizeHex(btn.dataset.paletteColor);
      state.pickerHex = color;
      pushManualUpdates([...state.selected], color, null);
    });
  });
}
function bindBrightness() {
  els.brightnessInput.addEventListener("input", () => {
    const value = Number(els.brightnessInput.value);
    state.brightness = value;
    els.brightnessValue.textContent = `${value}%`;
    els.brightnessInput.style.setProperty("--pct", `${value}%`);
  });
  els.brightnessInput.addEventListener("change", () => {
    const value = Number(els.brightnessInput.value);
    pushManualUpdates([...state.selected], null, value);
  });
}
function bindMaster() {
  els.masterSwitch.addEventListener("click", async () => {
    const enabled = els.masterSwitch.getAttribute("aria-pressed") !== "true";
    try {
      const result = await invoke("set_lights_enabled", { enabled });
      if (result.status === "err") showBanner(`Rejected: ${result.error}`);
    } catch (err) {
      showBanner(`Command failed: ${err}`);
    }
  });
}
function bindModeTabs() {
  els.modeTabs.forEach((tab) => {
    tab.addEventListener("click", async () => {
      const mode = tab.dataset.modeTab;
      try {
        const result = await invoke("set_mode", { mode });
        if (result.status === "err") showBanner(`Rejected: ${result.error}`);
      } catch (err) {
        showBanner(`Command failed: ${err}`);
      }
    });
  });
}
function bindContentAware() {
  els.chooseDisplay.addEventListener("click", async () => {
    els.chooseDisplay.disabled = true;
    try {
      const result = await invoke("choose_desktop_display");
      if (result.status === "err") showBanner(`Rejected: ${result.error}`);
    } catch (err) {
      showBanner(`Command failed: ${err}`);
    } finally {
      els.chooseDisplay.disabled = false;
    }
  });
  els.restartCapture.addEventListener("click", async () => {
    els.restartCapture.disabled = true;
    try {
      const result = await invoke("restart_capture");
      if (result.status === "err") showBanner(`Rejected: ${result.error}`);
    } catch (err) {
      showBanner(`Command failed: ${err}`);
    } finally {
      els.restartCapture.disabled = false;
    }
  });
}

// -- Setup page --------------------------------------------------------

function showOutcome(node, outcome) {
  if (!outcome) {
    node.hidden = true;
    return;
  }
  node.hidden = false;
  node.classList.remove("ok", "err");
  if (outcome.status === "ok") {
    node.classList.add("ok");
    node.textContent = outcome.message;
  } else {
    node.classList.add("err");
    node.textContent = outcome.error;
  }
}
function statusClass(good) {
  return good ? "ok" : "warn";
}
function statusText(good, yes = "yes", no = "no") {
  return good ? yes : no;
}

async function refreshSetup() {
  await Promise.all([loadUdev(), loadService(), loadGamingService()]);
}
async function loadUdev() {
  try {
    const status = await invoke("check_udev_status");
    state.udev = status;
    els.udevRule.textContent = statusText(status.rule_present, "installed", "missing");
    els.udevRule.className = statusClass(status.rule_present);
    els.udevDevice.textContent = statusText(status.device_present, "detected", "not detected");
    els.udevDevice.className = statusClass(status.device_present);
    els.udevWritable.textContent = statusText(
      status.device_writable,
      "yes",
      status.device_present ? "no (rule may not have applied yet)" : "n/a",
    );
    els.udevWritable.className = statusClass(status.device_writable);
  } catch (err) {
    console.warn("check_udev_status failed", err);
  }
}
async function loadService() {
  try {
    const status = await invoke("check_service_status");
    state.serviceUnit = status;
    els.serviceInstalled.textContent = statusText(status.unit_installed, "yes", "no");
    els.serviceInstalled.className = statusClass(status.unit_installed);
    els.serviceEnabled.textContent = statusText(status.enabled, "yes", "no");
    els.serviceEnabled.className = statusClass(status.enabled);
    els.serviceActive.textContent = statusText(status.active, "running", "stopped");
    els.serviceActive.className = statusClass(status.active);
    els.servicePath.textContent = status.unit_path || "—";
  } catch (err) {
    console.warn("check_service_status failed", err);
  }
}

async function loadGamingService() {
  try {
    const status = await invoke("check_gaming_service_status");
    els.gamingInstalled.textContent = statusText(status.unit_installed, "yes", "no");
    els.gamingInstalled.className = statusClass(status.unit_installed);
    els.gamingEnabled.textContent = statusText(status.enabled, "yes", "no");
    els.gamingEnabled.className = statusClass(status.enabled);
    els.gamingActive.textContent = statusText(status.active, "running", "stopped");
    els.gamingActive.className = statusClass(status.active);
  } catch (err) {
    console.warn("check_gaming_service_status failed", err);
  }
}

function bindSetup() {
  els.installUdev.addEventListener("click", async () => {
    els.installUdev.disabled = true;
    showOutcome(els.udevOutcome, null);
    try {
      const outcome = await invoke("install_udev_rule");
      showOutcome(els.udevOutcome, outcome);
    } catch (err) {
      showOutcome(els.udevOutcome, { status: "err", error: String(err) });
    } finally {
      els.installUdev.disabled = false;
      await loadUdev();
    }
  });
  els.refreshUdev.addEventListener("click", loadUdev);
  els.installService.addEventListener("click", async () => {
    els.installService.disabled = true;
    showOutcome(els.serviceOutcome, null);
    try {
      const outcome = await invoke("install_service_unit");
      showOutcome(els.serviceOutcome, outcome);
    } catch (err) {
      showOutcome(els.serviceOutcome, { status: "err", error: String(err) });
    } finally {
      els.installService.disabled = false;
      await loadService();
    }
  });
  const bindServiceAction = (btn, action) => {
    btn.addEventListener("click", async () => {
      btn.disabled = true;
      try {
        const outcome = await invoke("service_action", { action });
        showOutcome(els.serviceOutcome, outcome);
      } catch (err) {
        showOutcome(els.serviceOutcome, { status: "err", error: String(err) });
      } finally {
        btn.disabled = false;
        await loadService();
      }
    });
  };
  bindServiceAction(els.serviceStart, "start");
  bindServiceAction(els.serviceStop, "stop");
  bindServiceAction(els.serviceRestart, "restart");
  els.refreshService.addEventListener("click", loadService);

  els.uninstallService.addEventListener("click", async () => {
    els.uninstallService.disabled = true;
    showOutcome(els.serviceOutcome, null);
    try {
      const outcome = await invoke("uninstall_service_unit");
      showOutcome(els.serviceOutcome, outcome);
    } catch (err) {
      showOutcome(els.serviceOutcome, { status: "err", error: String(err) });
    } finally {
      els.uninstallService.disabled = false;
      await loadService();
    }
  });

  els.installGaming.addEventListener("click", async () => {
    els.installGaming.disabled = true;
    showOutcome(els.gamingOutcome, null);
    try {
      const outcome = await invoke("install_gaming_service_unit");
      showOutcome(els.gamingOutcome, outcome);
    } catch (err) {
      showOutcome(els.gamingOutcome, { status: "err", error: String(err) });
    } finally {
      els.installGaming.disabled = false;
      await loadGamingService();
    }
  });

  els.uninstallGaming.addEventListener("click", async () => {
    els.uninstallGaming.disabled = true;
    showOutcome(els.gamingOutcome, null);
    try {
      const outcome = await invoke("uninstall_gaming_service_unit");
      showOutcome(els.gamingOutcome, outcome);
    } catch (err) {
      showOutcome(els.gamingOutcome, { status: "err", error: String(err) });
    } finally {
      els.uninstallGaming.disabled = false;
      await loadGamingService();
    }
  });

  els.refreshGaming.addEventListener("click", loadGamingService);

  els.relinkLauncher.addEventListener("click", async () => {
    els.relinkLauncher.disabled = true;
    try {
      const outcome = await invoke("relink_launcher");
      if (outcome.status === "ok") {
        els.launcherBanner.hidden = true;
      } else {
        alert(`Re-link failed: ${outcome.error}`);
      }
    } catch (err) {
      alert(`Re-link failed: ${String(err)}`);
    } finally {
      els.relinkLauncher.disabled = false;
    }
  });
}

// -- Diagnostics page --------------------------------------------------

function renderDiagnostics() {
  const snap = state.snapshot;
  const setValue = (el, value, kls) => {
    el.textContent = value == null || value === "" ? "—" : String(value);
    el.className = "metric-value" + (kls ? " " + kls : "");
  };
  if (!snap) {
    ["diagService", "diagWriter", "diagCapture", "diagMode", "diagBackend"].forEach((key) =>
      setValue(els[key], "—", ""),
    );
    return;
  }
  setValue(els.diagService, snap.service_health, healthClass(snap.service_health));
  setValue(els.diagWriter, snap.writer_health, healthClass(snap.writer_health));
  setValue(els.diagCapture, snap.capture_health, healthClass(snap.capture_health));
  setValue(els.diagMode, snap.mode, "");
  setValue(els.diagBackend, backendLabel(snap.capture_backend), "");
  setValue(els.diagFrames, snap.diagnostics.captured_frames, "");
  setValue(els.diagNewest, snap.diagnostics.newest_value_replacements, "");
  setValue(
    els.diagStalls,
    snap.diagnostics.capture_stalls,
    snap.diagnostics.capture_stalls > 0 ? "warn" : "",
  );
  setValue(
    els.diagUsbfail,
    snap.diagnostics.usb_report_failures,
    snap.diagnostics.usb_report_failures > 0 ? "warn" : "",
  );
  setValue(
    els.diagUsbrec,
    snap.diagnostics.usb_recoveries,
    snap.diagnostics.usb_recoveries > 0 ? "warn" : "",
  );
  const rate = snap.diagnostics.capture_rate_millihertz;
  setValue(els.diagRate, rate > 0 ? `${(rate / 1000).toFixed(1)} fps` : "—", "");
  setValue(els.diagRevision, snap.revision, "");
}

function bindDiagnostics() {
  els.copyDiag.addEventListener("click", async () => {
    const snap = state.snapshot;
    if (!snap) return;
    const lines = [
      `G560 Linux Utility diagnostic report`,
      `revision=${snap.revision} mode=${snap.mode} lights_enabled=${snap.lights_enabled}`,
      `backend=${backendLabel(snap.capture_backend)}`,
      `service_health=${snap.service_health} writer_health=${snap.writer_health} capture_health=${snap.capture_health} device_health=${snap.device_health}`,
      `captured_frames=${snap.diagnostics.captured_frames} newest_replacements=${snap.diagnostics.newest_value_replacements}`,
      `capture_stalls=${snap.diagnostics.capture_stalls}`,
      `usb_report_failures=${snap.diagnostics.usb_report_failures} usb_recoveries=${snap.diagnostics.usb_recoveries}`,
      `capture_rate_mHz=${snap.diagnostics.capture_rate_millihertz}`,
      `pending=${snap.pending}`,
      `api_version=${snap.api_version}`,
    ];
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      els.copyFeedback.textContent = "Copied.";
    } catch (err) {
      els.copyFeedback.textContent = `Copy failed: ${err}`;
    }
    setTimeout(() => (els.copyFeedback.textContent = ""), 2000);
  });
}

// -- About page --------------------------------------------------------

async function loadVersion() {
  if (state.version) {
    fillVersion(state.version);
    return;
  }
  try {
    const v = await invoke("version_info");
    state.version = v;
    fillVersion(v);
  } catch (err) {
    console.warn("version_info failed", err);
  }
}
function fillVersion(v) {
  els.aboutGui.textContent = v.gui_version;
  els.aboutApi.textContent = v.api_version;
  els.aboutBus.textContent = v.bus_name;
  els.aboutTarget.textContent = v.build_target;
  els.aboutDistro.textContent = v.distribution || "unknown";
  els.aboutSession.textContent = v.session_type || "unknown";
  els.aboutDesktop.textContent = v.desktop || "unknown";
  els.aboutKernel.textContent = v.kernel || "unknown";
}
function bindAbout() {
  els.copyVersion.addEventListener("click", async () => {
    const v = state.version;
    if (!v) return;
    const text = [
      `G560 Linux Utility GUI ${v.gui_version} (API ${v.api_version})`,
      `Bus: ${v.bus_name}`,
      `Interface: ${v.interface_name}`,
      `Build target: ${v.build_target}`,
      `Distribution: ${v.distribution || "unknown"}`,
      `Session: ${v.session_type || "unknown"}`,
      `Desktop: ${v.desktop || "unknown"}`,
      `Kernel: ${v.kernel || "unknown"}`,
    ].join("\n");
    try {
      await navigator.clipboard.writeText(text);
      els.versionFeedback.textContent = "Copied.";
    } catch (err) {
      els.versionFeedback.textContent = `Copy failed: ${err}`;
    }
    setTimeout(() => (els.versionFeedback.textContent = ""), 2000);
  });
}

// -- bootstrap ---------------------------------------------------------

async function bootstrap() {
  bindRail();
  bindZones();
  bindGroups();
  bindColor();
  bindBrightness();
  bindMaster();
  bindModeTabs();
  bindContentAware();
  bindSetup();
  bindDiagnostics();
  bindAbout();

  firstRunStart.addEventListener("click", async () => {
    switchPage("setup");
    await markSetupComplete();
  });
  firstRunSkip.addEventListener("click", markSetupComplete);

  switchPage("lighting");

  try {
    await listen("snapshot-changed", (event) => {
      state.snapshot = event.payload;
      render();
    });
  } catch (err) {
    report(`listen snapshot-changed failed: ${err}`);
  }
  try {
    await listen("connection-state", (event) => {
      state.connection = event.payload;
      render();
    });
  } catch (err) {
    report(`listen connection-state failed: ${err}`);
  }
  try {
    await listen("launcher-status", (event) => {
      const payload = event.payload;
      if (!payload || payload.context !== "appimage") return;
      const state = payload.state;
      if (state && state.state === "stale") {
        els.launcherOldPath.textContent = state.embedded || "";
        els.launcherBanner.hidden = false;
      } else {
        els.launcherBanner.hidden = true;
      }
    });
  } catch (err) {
    report(`listen launcher-status failed: ${err}`);
  }

  try {
    state.connection = await invoke("get_connection_state");
  } catch (err) {
    report(`get_connection_state failed: ${err}`);
    state.connection = { kind: "disconnected", reason: String(err) };
  }
  try {
    const cached = await invoke("get_cached_snapshot");
    if (cached) state.snapshot = cached;
  } catch (err) {
    report(`get_cached_snapshot failed: ${err}`);
  }
  render();
}

bootstrap();
