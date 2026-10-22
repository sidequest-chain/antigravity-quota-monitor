const { invoke } = window.__TAURI__.core;
const { getCurrentWindow, LogicalSize } = window.__TAURI__.window;

const appWindow = getCurrentWindow();

let lastSnapshot = null;
let showRemaining = true;
let isCompact = false;
let isPinned = true;
let isCheckingSession = false;

let manifest = { defaultLocale: "en-US", locales: [] };
let currentLocale = "en-US";
let dict = {};

const statusDot = document.getElementById("statusDot");
const tierBadge = document.getElementById("tierBadge");
const summaryList = document.getElementById("summaryList");
const modelsList = document.getElementById("modelsList");
const compactView = document.getElementById("compactView");
const compactPills = document.getElementById("compactPills");
const fullView = document.getElementById("fullView");
const loginView = document.getElementById("loginView");
const loginTitle = document.getElementById("loginTitle");
const loginDesc = document.getElementById("loginDesc");
const authedContent = document.getElementById("authedContent");
const btnGoogleLogin = document.getElementById("btnGoogleLogin");
const loginBtnText = document.getElementById("loginBtnText");
const btnSwitchAccount = document.getElementById("btnSwitchAccount");
const errorBanner = document.getElementById("errorBanner");
const userEmail = document.getElementById("userEmail");
const sessionsCount = document.getElementById("sessionsCount");
const updatedAt = document.getElementById("updatedAt");
const modeLabel = document.getElementById("modeLabel");
const btnRefresh = document.getElementById("btnRefresh");
const btnPin = document.getElementById("btnPin");
const btnCompact = document.getElementById("btnCompact");
const btnClose = document.getElementById("btnClose");
const btnToggleMode = document.getElementById("btnToggleMode");
const opacitySlider = document.getElementById("opacitySlider");
const langSelect = document.getElementById("langSelect");

function t(keyPath, vars = {}) {
  const parts = keyPath.split(".");
  let cur = dict;
  for (const p of parts) {
    if (cur && typeof cur === "object" && p in cur) {
      cur = cur[p];
    } else {
      return keyPath;
    }
  }
  if (typeof cur !== "string") return keyPath;
  return cur.replace(/\{(\w+)\}/g, (_, k) => (k in vars ? String(vars[k]) : `{${k}}`));
}

function resolveInitialLocale(availableCodes, fallback) {
  const saved = localStorage.getItem("agm-lang");
  if (saved && availableCodes.includes(saved)) {
    return saved;
  }
  const nav = (navigator.language || "").trim();
  if (availableCodes.includes(nav)) {
    return nav;
  }
  const base = nav.split("-")[0].toLowerCase();
  const match = availableCodes.find((c) => c.toLowerCase() === base || c.toLowerCase().startsWith(base + "-"));
  return match || fallback;
}

async function initI18n() {
  const res = await fetch("locales/manifest.json");
  manifest = await res.json();
  const codes = manifest.locales.map((l) => l.code);
  currentLocale = resolveInitialLocale(codes, manifest.defaultLocale);

  langSelect.innerHTML = manifest.locales
    .map((l) => `<option value="${l.code}">${l.code.toUpperCase()}</option>`)
    .join("");
  langSelect.value = currentLocale;

  await loadLocale(currentLocale);
}

async function loadLocale(code) {
  const res = await fetch(`locales/${code}.json`);
  dict = await res.json();
  currentLocale = code;
  localStorage.setItem("agm-lang", code);
  document.documentElement.lang = code;

  document.querySelectorAll("[data-i18n]").forEach((el) => {
    const key = el.getAttribute("data-i18n");
    el.textContent = t(key);
  });

  document.querySelectorAll("[data-i18n-title]").forEach((el) => {
    const key = el.getAttribute("data-i18n-title");
    el.title = t(key);
  });

  modeLabel.textContent = showRemaining ? t("mode.remaining") : t("mode.used");

  try {
    await invoke("set_tray_labels", {
      toggle: t("tray.toggle"),
      top: t("tray.top"),
      quit: t("tray.quit"),
    });
  } catch (_) {}

  if (lastSnapshot) {
    renderSnapshot(lastSnapshot);
  }
}

function colorClassForRemaining(remFraction) {
  if (remFraction <= 0.2) return "fill-danger";
  if (remFraction <= 0.5) return "fill-warn";
  return "fill-good";
}

function textColorForRemaining(remFraction) {
  if (remFraction <= 0.2) return "#fb7185";
  if (remFraction <= 0.5) return "#fbbf24";
  return "#34d399";
}

function formatPct(remPct, usedPct) {
  const val = showRemaining ? remPct : usedPct;
  return `${val.toFixed(1).replace(".0", "")}%`;
}

function formatResetSeconds(secs) {
  if (secs === null || secs === undefined) {
    return t("time.none");
  }
  if (secs <= 0) {
    return t("time.now");
  }
  const days = Math.floor(secs / 86400);
  const hours = Math.floor((secs % 86400) / 3600);
  const mins = Math.floor((secs % 3600) / 60);

  if (days > 0) {
    return t("time.daysHours", { d: days, h: hours });
  }
  if (hours > 0) {
    return t("time.hoursMins", { h: hours, m: mins });
  }
  if (mins > 0) {
    return t("time.mins", { m: mins });
  }
  return t("time.secs", { s: secs });
}

function formatError(errKey) {
  const raw = String(errKey);
  if (dict.errors && raw in dict.errors) {
    return t(`errors.${raw}`);
  }
  return t("errors.UNKNOWN", { error: raw });
}

function renderCard(title, subtitle, remFraction, remPct, usedPct, resetSeconds) {
  const barWidth = showRemaining ? remPct : usedPct;
  const fillClass = colorClassForRemaining(remFraction);
  const textColor = textColorForRemaining(remFraction);
  const resetIn = formatResetSeconds(resetSeconds);

  return `
    <div class="quota-card">
      <div class="card-top">
        <div>
          <div class="card-title">${title}</div>
          ${subtitle ? `<div class="card-sub">${subtitle}</div>` : ""}
        </div>
        <div class="card-stats">
          <span class="reset-badge" title="${t("tooltips.resetTimer")}">⏱ ${resetIn}</span>
          <span class="pct-badge" style="color: ${textColor}">${formatPct(remPct, usedPct)}</span>
        </div>
      </div>
      <div class="progress-track">
        <div class="progress-fill ${fillClass}" style="width: ${Math.max(2, barWidth)}%"></div>
      </div>
    </div>
  `;
}

function resolveGroupTitle(bucket) {
  if (bucket.groupKey === "gemini") return t("groups.gemini");
  if (bucket.groupKey === "claude_gpt") return t("groups.claude_gpt");
  return bucket.groupName || t("groups.other");
}

function resolveWindowTitle(bucket) {
  if (bucket.window === "5h") return t("windows.5h");
  if (bucket.window === "weekly") return t("windows.weekly");
  return bucket.displayName;
}

function renderSnapshot(snap) {
  if (!snap) return;

  if (snap.authenticated) {
    tierBadge.textContent = snap.paidTierName || snap.tierName || t("brandTitle");
    userEmail.textContent = snap.email;
    btnSwitchAccount.title = t("tooltips.accountConnected", {
      source: snap.authSource || t("brandTitle"),
    });
  } else {
    tierBadge.textContent = snap.installed ? t("status.notSignedIn") : t("status.missingInstall");
    userEmail.textContent = t("status.notConnected");
    btnSwitchAccount.title = t("tooltips.accountDisconnected");
  }

  sessionsCount.textContent = t("status.activeSessions", { count: snap.activeSessions });
  updatedAt.textContent = snap.updatedAt;

  if (!snap.authenticated) {
    authedContent.classList.add("hidden");
    loginView.classList.remove("hidden");
    if (!snap.installed) {
      loginTitle.textContent = t("onboarding.missingTitle");
      loginDesc.textContent = t("onboarding.missingDesc");
      loginBtnText.textContent = isCheckingSession
        ? t("status.checkingSession")
        : t("onboarding.missingAction");
    } else {
      loginTitle.textContent = t("onboarding.loginTitle");
      loginDesc.textContent = t("onboarding.loginDesc");
      loginBtnText.textContent = isCheckingSession
        ? t("status.checkingSession")
        : t("onboarding.loginAction");
    }
    return;
  }

  loginView.classList.add("hidden");
  authedContent.classList.remove("hidden");

  const infoEl = document.getElementById("infoSessionDetails");
  if (infoEl) {
    infoEl.innerHTML = `
      <div><strong>${t("info.sourceLabel")}</strong> ${snap.authSource || "—"}</div>
      <div><strong>${t("info.accountLabel")}</strong> ${snap.email || "—"}</div>
      <div><strong>${t("info.projectLabel")}</strong> ${snap.projectId || "—"}</div>
    `;
  }

  // 1. Summary view: Grouped by Quota Group
  let summaryHtml = "";
  let currentGroup = "";

  if (snap.buckets && snap.buckets.length > 0) {
    for (const b of snap.buckets) {
      const grpLabel = resolveGroupTitle(b);
      if (grpLabel !== currentGroup) {
        currentGroup = grpLabel;
        summaryHtml += `<div class="section-label">${currentGroup}</div>`;
      }
      summaryHtml += renderCard(
        resolveWindowTitle(b),
        "",
        b.remainingFraction,
        b.remainingPercent,
        b.usedPercent,
        b.resetSeconds
      );
    }
  }

  summaryList.innerHTML = summaryHtml || `<div class="skeleton">${t("status.noQuotas")}</div>`;

  // 2. Detailed Models view
  let modelsHtml = "";
  let currentFam = "";
  for (const m of snap.models || []) {
    const famLabel = t(`families.${m.familyKey}`);
    if (famLabel !== currentFam) {
      currentFam = famLabel;
      modelsHtml += `<div class="section-label">${currentFam}</div>`;
    }
    modelsHtml += renderCard(
      m.displayName,
      "",
      m.remainingFraction,
      m.remainingPercent,
      m.usedPercent,
      m.resetSeconds
    );
  }
  modelsList.innerHTML = modelsHtml || `<div class="skeleton">${t("status.noModels")}</div>`;

  // 3. Compact Mini-Pills view
  const miniMap = [
    { id: "gemini-5h", labelKey: "mini.gemini_5h" },
    { id: "gemini-weekly", labelKey: "mini.gemini_weekly" },
    { id: "3p-5h", labelKey: "mini.claude_5h" },
    { id: "3p-weekly", labelKey: "mini.claude_weekly" },
  ];

  const miniItems = [];
  for (const m of miniMap) {
    const found = (snap.buckets || []).find((b) => b.id === m.id);
    if (found) {
      miniItems.push({ shortLabel: t(m.labelKey), ...found });
    }
  }

  compactPills.innerHTML = miniItems
    .map((item) => {
      const barWidth = showRemaining ? item.remainingPercent : item.usedPercent;
      const fillClass = colorClassForRemaining(item.remainingFraction);
      const textColor = textColorForRemaining(item.remainingFraction);
      const grpLabel = resolveGroupTitle(item);
      const winLabel = resolveWindowTitle(item);
      const resetIn = formatResetSeconds(item.resetSeconds);
      return `
        <div class="mini-pill" title="${grpLabel} - ${winLabel} (${resetIn})">
          <div class="mini-top">
            <span class="mini-name">${item.shortLabel}</span>
            <span style="color:${textColor}">${formatPct(item.remainingPercent, item.usedPercent)}</span>
          </div>
          <div class="progress-track" style="height:4px">
            <div class="progress-fill ${fillClass}" style="width:${Math.max(2, barWidth)}%"></div>
          </div>
        </div>
      `;
    })
    .join("");
}

async function refreshUsage() {
  statusDot.className = "status-dot loading";
  btnRefresh.classList.add("spinning");
  errorBanner.classList.add("hidden");

  try {
    const snap = await invoke("get_usage");
    lastSnapshot = snap;
    renderSnapshot(snap);
    statusDot.className = "status-dot";
  } catch (err) {
    statusDot.className = "status-dot error";
    errorBanner.textContent = formatError(err);
    errorBanner.classList.remove("hidden");
  } finally {
    btnRefresh.classList.remove("spinning");
  }
}

async function startGoogleLogin() {
  statusDot.className = "status-dot loading";
  errorBanner.classList.add("hidden");
  btnGoogleLogin.disabled = true;
  isCheckingSession = true;
  loginBtnText.textContent = t("status.checkingSession");

  try {
    const snap = await invoke("launch_antigravity");
    lastSnapshot = snap;
    renderSnapshot(snap);
    statusDot.className = "status-dot";
  } catch (err) {
    statusDot.className = "status-dot error";
    errorBanner.textContent = formatError(err);
    errorBanner.classList.remove("hidden");
  } finally {
    isCheckingSession = false;
    btnGoogleLogin.disabled = false;
    if (lastSnapshot) {
      renderSnapshot(lastSnapshot);
    }
  }
}

// Event Listeners
btnRefresh.addEventListener("click", () => refreshUsage());
btnGoogleLogin.addEventListener("click", () => startGoogleLogin());
btnSwitchAccount.addEventListener("click", () => startGoogleLogin());

langSelect.addEventListener("change", (e) => {
  loadLocale(e.target.value);
});

btnToggleMode.addEventListener("click", () => {
  showRemaining = !showRemaining;
  modeLabel.textContent = showRemaining ? t("mode.remaining") : t("mode.used");
  if (lastSnapshot) renderSnapshot(lastSnapshot);
});

btnPin.addEventListener("click", async () => {
  isPinned = !isPinned;
  await appWindow.setAlwaysOnTop(isPinned);
  btnPin.classList.toggle("active", isPinned);
});

btnCompact.addEventListener("click", async () => {
  isCompact = !isCompact;
  btnCompact.classList.toggle("active", isCompact);
  if (isCompact) {
    fullView.classList.add("hidden");
    compactView.classList.remove("hidden");
    await appWindow.setSize(new LogicalSize(360, 76));
  } else {
    compactView.classList.add("hidden");
    fullView.classList.remove("hidden");
    await appWindow.setSize(new LogicalSize(340, 410));
  }
});

btnClose.addEventListener("click", async () => {
  await appWindow.hide();
});

opacitySlider.addEventListener("input", (e) => {
  const val = Number(e.target.value) / 100;
  document.documentElement.style.setProperty("--bg-alpha", val.toString());
});

document.querySelectorAll(".tab").forEach((tabBtn) => {
  tabBtn.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((b) => b.classList.remove("active"));
    tabBtn.classList.add("active");
    const target = tabBtn.getAttribute("data-tab");
    document.getElementById("tabSummary").classList.toggle("hidden", target !== "summary");
    document.getElementById("tabModels").classList.toggle("hidden", target !== "models");
    document.getElementById("tabInfo").classList.toggle("hidden", target !== "info");
  });
});

// Initialize locales first, then load usage & start 60s auto-refresh
initI18n().then(() => {
  refreshUsage();
  setInterval(refreshUsage, 60000);
});
