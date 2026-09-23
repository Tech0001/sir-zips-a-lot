const $ = (id) => document.getElementById(id);
const invoke = window.__TAURI__?.core.invoke;
let busy = false;
let status;
let initialized = false;
const renderedIds = new Set();
const systemTheme = window.matchMedia("(prefers-color-scheme: dark)");
let chosenTheme;
try {
  const saved = localStorage.getItem("sir-zips-a-lot-theme");
  if (saved === "light" || saved === "dark") chosenTheme = saved;
} catch {
  /* A theme choice still works if local storage is unavailable. */
}

function applyTheme() {
  const theme = chosenTheme ?? (systemTheme.matches ? "dark" : "light");
  document.documentElement.dataset.theme = theme;
  const label = `Switch to ${theme === "dark" ? "light" : "dark"} mode`;
  $("theme-toggle").setAttribute("aria-label", label);
  $("theme-toggle").title = label;
  const nativeWindow = window.__TAURI__?.window?.getCurrentWindow();
  nativeWindow?.setTheme(chosenTheme ?? null).catch(() => {
    // Some desktop environments own the title-bar theme themselves.
  });
}

$("theme-toggle").addEventListener("click", () => {
  chosenTheme =
    document.documentElement.dataset.theme === "dark" ? "light" : "dark";
  try {
    localStorage.setItem("sir-zips-a-lot-theme", chosenTheme);
  } catch {
    /* Keep the in-memory preference. */
  }
  applyTheme();
});
systemTheme.addEventListener("change", () => {
  if (!chosenTheme) applyTheme();
});
applyTheme();

function showError(error) {
  $("error").textContent = String(error);
  $("error").hidden = false;
}

function render(next) {
  status = next;
  if (!initialized) {
    if (next.config) {
      $("source").value = next.config.source;
      $("destination").value = next.config.destination;
      $("quiet-seconds").value = next.config.quietSeconds;
      $("poll-seconds").value = next.config.pollSeconds;
    }
    initialized = true;
  }
  $("settings").disabled = next.running || busy;
  $("start").hidden = next.running;
  $("start").disabled = busy;
  $("stop").hidden = !next.running;
  $("stop").disabled = next.stopping || busy;
  $("stop").textContent = next.stopping
    ? "Finishing current order…"
    : "Stop watching";
  $("badge").textContent = next.stopping
    ? "Clocking out"
    : next.running
      ? "On duty"
      : "Off duty";
  $("badge").className =
    `badge ${next.stopping ? "stopping" : next.running ? "active" : ""}`;
  $("status-dot").className = `dot ${next.running ? "active" : ""}`;
  $("delivered").textContent = next.delivered;
  $("pending").textContent = next.pending;
  $("status-title").textContent = next.stopping
    ? "Finishing up"
    : next.running
      ? "Keeping watch"
      : "Ready when you are";
  $("status-detail").textContent = next.stopping
    ? "Stopping after the current order."
    : next.running
      ? `Checking for orders every ${next.config.pollSeconds} ${next.config.pollSeconds === 1 ? "second" : "seconds"}.`
      : "Press Start to put your knight on duty.";
  $("activity-empty").hidden = next.activity.length > 0;
  const currentIds = new Set(next.activity.map((entry) => entry.id));
  for (const child of Array.from($("activity-list").children)) {
    const id = Number(child.dataset.id);
    if (!currentIds.has(id)) {
      child.remove();
      renderedIds.delete(id);
    }
  }
  for (const entry of next.activity) {
    if (renderedIds.has(entry.id)) continue;
    const row = document.createElement("li");
    row.dataset.id = entry.id;
    row.className = `${entry.kind}-event`;
    const time = document.createElement("time");
    const date = new Date(entry.timestamp * 1000);
    time.dateTime = date.toISOString();
    time.textContent = date.toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
    });
    const message = document.createElement("span");
    message.textContent = entry.message;
    row.append(time, message);
    $("activity-list").prepend(row);
    renderedIds.add(entry.id);
  }
}

async function refresh() {
  render(await invoke("get_status"));
}

document.querySelectorAll(".browse").forEach((button) => {
  button.addEventListener("click", async () => {
    try {
      const path = await window.__TAURI__.dialog.open({
        directory: true,
        multiple: false,
        title:
          button.dataset.target === "source"
            ? "Choose the orders folder"
            : "Choose the ZIP destination",
      });
      if (path) $(button.dataset.target).value = path;
    } catch (error) {
      showError(error);
    }
  });
});

$("watch-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (busy || status?.running) return;
  busy = true;
  $("error").hidden = true;
  if (status) render(status);
  try {
    await invoke("start_watch", {
      config: {
        source: $("source").value.trim(),
        destination: $("destination").value.trim(),
        quietSeconds: Number($("quiet-seconds").value),
        pollSeconds: Number($("poll-seconds").value),
      },
    });
  } catch (error) {
    showError(error);
  } finally {
    busy = false;
    await refresh().catch(showError);
  }
});

$("stop").addEventListener("click", async () => {
  busy = true;
  if (status) render(status);
  try {
    await invoke("stop_watch");
  } catch (error) {
    showError(error);
  } finally {
    busy = false;
    await refresh().catch(showError);
  }
});

async function poll() {
  try {
    await refresh();
  } catch (error) {
    showError(error);
  } finally {
    setTimeout(poll, 1000);
  }
}

if (invoke) poll();
else {
  $("settings").disabled = true;
  $("start").disabled = true;
  showError("Open Sir Zips-a-Lot as a desktop app to watch folders.");
}
