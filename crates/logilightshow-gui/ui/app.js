// LogiLightShow GUI — Lighting page behaviour.
// The frontend keeps a shadow copy of the last confirmed snapshot from the
// service and re-renders on `snapshot-changed`. All user actions dispatch
// through Tauri commands and never write hardware directly.

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;

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
};

const panel = document.querySelector("[data-interactive-lighting]");
const els = {
  serviceDot: panel.querySelector("[data-service-dot]"),
  serviceLabel: panel.querySelector("[data-service-label]"),
  deviceDot: panel.querySelector("[data-device-dot]"),
  deviceLabel: panel.querySelector("[data-device-label]"),
  masterSwitch: panel.querySelector("[data-master-switch]"),
  masterLabel: panel.querySelector("[data-master-label]"),
  modeTabs: panel.querySelectorAll("[data-mode-tab]"),
  modeBadge: panel.querySelector("[data-mode-badge]"),
  liveLabel: panel.querySelector("[data-live-label]"),
  controls: panel.querySelector(".controls"),
  banner: panel.querySelector("[data-banner]"),
  bannerText: panel.querySelector("[data-banner-text]"),
  zones: panel.querySelectorAll("[data-zone]"),
  groups: panel.querySelectorAll("[data-group]"),
  colorInput: panel.querySelector("[data-color-input]"),
  paletteButtons: panel.querySelectorAll("[data-palette-color]"),
  hexOutput: panel.querySelector("[data-hex-output]"),
  rgbOutput: panel.querySelector("[data-rgb-output]"),
  selectionCount: panel.querySelector("[data-selection-count]"),
  selectionCopy: panel.querySelector("[data-selection-copy]"),
  brightnessInput: panel.querySelector("[data-brightness-input]"),
  brightnessValue: panel.querySelector("[data-brightness-value]"),
  lightZones: {
    "left-rear": panel.querySelectorAll('[data-light-zone="left-rear"]'),
    "left-front": panel.querySelectorAll('[data-light-zone="left-front"]'),
    "right-front": panel.querySelectorAll('[data-light-zone="right-front"]'),
    "right-rear": panel.querySelectorAll('[data-light-zone="right-rear"]'),
  },
};

function normalizeHex(value) {
  const upper = String(value).trim().toUpperCase();
  const match = /^#?([0-9A-F]{6})$/.exec(upper);
  if (!match) throw new Error(`invalid hex color: ${value}`);
  return `#${match[1]}`;
}

function hexToRgb(hex) {
  const normalized = normalizeHex(hex).slice(1);
  return {
    r: parseInt(normalized.slice(0, 2), 16),
    g: parseInt(normalized.slice(2, 4), 16),
    b: parseInt(normalized.slice(4, 6), 16),
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
  if (!entry) return "#14C8F4";
  return rgbToHex(entry.color);
}

function manualSettingForZone(zone) {
  if (!state.snapshot) return null;
  return state.snapshot.manual_zones.find((z) => z.zone === zone) || null;
}

function mixedSelection(getter) {
  const values = [...state.selected].map((zone) => getter(zone));
  if (values.length === 0) return null;
  const first = values[0];
  const mixed = values.some((v) => JSON.stringify(v) !== JSON.stringify(first));
  return mixed ? "mixed" : first;
}

function firstSelected() {
  return state.selected.values().next().value || null;
}

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
  els.modeBadge.textContent =
    mode === "content-aware" ? "Content-Aware mode" : "Manual mode";
  els.liveLabel.textContent =
    mode === "content-aware" ? "Live capture" : "Newest value only";
  els.controls.classList.toggle(
    "locked",
    mode === "content-aware" ||
      (state.snapshot && !state.snapshot.lights_enabled),
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
    els.colorInput.value = state.pickerHex;
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
  els.brightnessInput.style.setProperty(
    "--pct",
    `${els.brightnessInput.value}%`,
  );
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

function zoneLabel(zone) {
  return zone
    .split("-")
    .map((w) => w[0].toUpperCase() + w.slice(1))
    .join(" ");
}

function showBanner(text) {
  els.bannerText.textContent = text;
  els.banner.hidden = false;
}
function hideBanner() {
  els.banner.hidden = true;
}

async function pushManualUpdates(zones, color, brightness) {
  if (zones.length === 0) return;
  const updates = zones.map((zone) => ({
    zone,
    color: color != null ? hexToApiColor(color) : currentZoneColor(zone),
    brightness: brightness != null ? brightness : currentZoneBrightness(zone),
  }));
  try {
    const result = await invoke("set_manual_zones", { updates });
    if (result.status === "err") {
      showBanner(`Rejected: ${result.error}`);
    }
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

async function bootstrap() {
  bindZones();
  bindGroups();
  bindColor();
  bindBrightness();
  bindMaster();
  bindModeTabs();

  await listen("snapshot-changed", (event) => {
    state.snapshot = event.payload;
    render();
  });
  await listen("connection-state", (event) => {
    state.connection = event.payload;
    render();
  });

  try {
    const conn = await invoke("get_connection_state");
    state.connection = conn;
  } catch (err) {
    state.connection = { kind: "disconnected", reason: String(err) };
  }
  try {
    const cached = await invoke("get_cached_snapshot");
    if (cached) state.snapshot = cached;
  } catch (err) {
    console.warn("get_cached_snapshot failed", err);
  }
  render();
}

bootstrap();
