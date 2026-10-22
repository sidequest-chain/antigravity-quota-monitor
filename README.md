# Antigravity Quota Monitor (`antigravity-quota-monitor`)

`antigravity-quota-monitor` is a Windows desktop utility built with Rust and Tauri v2 that displays current usage quotas, model limits, and reset timers for Google Antigravity CLI (`agy`) and Antigravity IDE.

---

## Why This Exists

Google Antigravity enforces rolling usage quotas across its supported AI model families — specifically a 5-hour rolling window (`5h`) and a 7-day rolling window (`weekly`), split between Gemini models (Gemini 4 Argon, Gemini 3.8 Flash) and third-party models (Claude Opus 5.5, Claude Sonnet 5.5).

Without a dedicated monitor, checking how much quota remains or when a depleted window resets requires interrupting your workflow to run `/usage` inside an active CLI session or navigating into IDE settings panels. If you have multiple terminal or editor sessions open simultaneously, it is easy to hit a rate limit unexpectedly in the middle of a task. This utility keeps a small, always-visible meter on the desktop (or in the Windows System Tray) that polls your current quota state every 60 seconds and shows either remaining or consumed percentages alongside the exact time until the next reset.

---

## How It Works

1. **Local Installation & Session Discovery:** On startup, the application checks whether Antigravity CLI or Antigravity IDE is installed on the machine and looks for an existing signed-in session in three local locations:
   - The Windows Credential Manager entry `gemini:antigravity` (created by `agy`).
   - The Antigravity IDE state database at `%APPDATA%\Antigravity IDE\User\globalStorage\state.vscdb` (or `.backup`).
   - The Antigravity ACP token file at `%USERPROFILE%\.gemini\antigravity-acp\acp_token.json`.
2. **Dynamic Token Refresh (Zero Bundled Secrets):** The application does not embed any Google OAuth `client_id` or `client_secret` in its source code or compiled executable, nor does it run a custom OAuth login web server. When a stored `access_token` expires, the backend reads the OAuth client identifier and secret directly from the user's locally installed `agy.exe` or `language_server_windows_x64.exe` binary on disk, exchanges the local `refresh_token` with `https://oauth2.googleapis.com/token`, and caches the resulting short-lived access token in memory.
3. **Quota & Model Retrieval:** Using the valid access token, the Rust backend queries the Antigravity quota endpoints (`loadCodeAssist`, `retrieveUserQuotaSummary`, and `fetchAvailableModels` on `https://daily-cloudcode-pa.googleapis.com`), counts active `.lock` files in `%USERPROFILE%\.gemini\antigravity-cli\presence`, and passes a structured snapshot to the Tauri WebView2 frontend.
4. **Desktop Presentation:** The frontend renders the snapshot in two user-selectable layouts:
   - **Full View (340×430 px):** Separate tabs for aggregated quota buckets (`Kvoodid`), individual model quotas (`Mudelid`), and connection metadata (`Info`), with a toggle between remaining percentage (`Alles %`) and used percentage (`Kasutatud %`), plus a background opacity slider.
   - **Compact Pill View (360×76 px):** A minimal horizontal strip showing four summary progress bars (`Gem 5h`, `Gem Nd`, `Claude 5h`, `Claude Nd`) for keeping pinned in a screen corner.

---

## Technical Architecture and Details

### Stack and Crate Dependencies

- **Backend (`src/main.rs`):** Rust (2021 edition) running on `tokio` and `tauri` v2 (`tray-icon` feature).
- **HTTP Client:** `reqwest` v0.12 configured with `rustls-tls` (no OpenSSL dependency) and a 12-second request timeout.
- **Credential Access:** Direct Win32 FFI calls via `windows-sys` (`Win32_Security_Credentials::CredReadW` and `CredFree`) to read generic credentials from the Windows Credential Manager without spawning external PowerShell or `cmdkey` processes.
- **Frontend (`ui/index.html`, `ui/app.js`, `ui/styles.css`):** Vanilla HTML, CSS, and JavaScript communicating with the Rust backend over Tauri IPC (`invoke("get_usage")` and `invoke("launch_antigravity")`). Zero frontend bundler or Node.js runtime dependencies are required.

### Project Structure

```text
.
├── Cargo.toml                 # Crate manifest, dependencies, and release LTO/strip profile
├── build.rs                   # Tauri build script (tauri_build::build())
├── tauri.conf.json            # Tauri v2 window geometry, transparency, and bundle settings
├── capabilities/
│   └── default.json           # Tauri v2 ACL permissions for window sizing and always-on-top
├── icons/
│   ├── icon.ico               # Windows executable and system tray icon
│   └── icon.png               # Application icon source
├── src/
│   └── main.rs                # Credential readers, binary OAuth scanner, API client, and tray loop
└── ui/
    ├── index.html             # Widget DOM structure (data-i18n attributes, zero hardcoded text)
    ├── app.js                 # Tauri IPC calls, i18n loader, 60-second polling timer, and view state logic
    ├── styles.css             # Translucent dark theme and layout rules
    └── locales/               # External JSON language packs (ET, EN-US, EN-GB, LV, LT, FI, DE, SV, PL, FR, ES, UK)
        ├── manifest.json
        ├── en-US.json
        ├── en-GB.json
        ├── et.json
        ├── lv.json
        ├── lt.json
        ├── fi.json
        ├── de.json
        ├── sv.json
        ├── pl.json
        ├── fr.json
        ├── es.json
        └── uk.json
```

### Key Internal Mechanisms in `src/main.rs`

- **`load_stored_credential()`:** Checks `gemini:antigravity` in Windows Credential Manager first. If absent, `load_ide_vscdb_credential()` scans `state.vscdb` for the `antigravityUnifiedStateSync.oauthToken` record, decodes its outer and inner Base64 protobuf payloads, and extracts `ya29.*` access tokens and `1//*` refresh tokens. Falls back to `acp_token.json` if present.
- **`extract_oauth_clients_from_binary()` & `refresh_with_local_oauth_client()`:** Scans the byte slice of the locally installed `%LOCALAPPDATA%\agy\bin\agy.exe` or `%LOCALAPPDATA%\Programs\Antigravity-IDE\resources\app\extensions\antigravity\bin\language_server_windows_x64.exe` for `.apps.googleusercontent.com` client IDs and `GOCSPX-` secrets. Once a pair succeeds against `oauth2.googleapis.com/token`, it is cached in `AppState::cached_oauth_client` for the lifetime of the process.
- **`fetch_usage_internal()`:** Resolves the user's `cloudaicompanionProject` ID (from memory cache, `~/.gemini/antigravity-cli/cache/default_project_id.txt`, or `v1internal:loadCodeAssist`), then executes `v1internal:retrieveUserQuotaSummary` and `v1internal:fetchAvailableModels` concurrently via `tokio::join!`.
- **`count_active_sessions()`:** Counts `.lock` files inside `%USERPROFILE%\.gemini\antigravity-cli\presence` to display how many CLI/ACP agent sessions are currently active.

---

## Prerequisites and Setup

### Runtime Prerequisites

1. **Operating System:** Windows 10 or Windows 11 (x86_64) with Microsoft Edge WebView2 Runtime installed (included by default on Windows 11 and modern Windows 10 systems).
2. **Antigravity Installation:** You must have either **Google Antigravity CLI (`agy`)** or **Antigravity IDE** installed and signed in on your machine:
   - CLI binary path checked: `%LOCALAPPDATA%\agy\bin\agy.exe`
   - IDE binary path checked: `%LOCALAPPDATA%\Programs\Antigravity-IDE\Antigravity IDE.exe`

### Building from Source

To compile the standalone release executable on Windows, you need the Rust toolchain (`rustup` + `cargo`) with the `x86_64-pc-windows-msvc` target and Visual Studio C++ Build Tools.

```powershell
# 1. Clone the repository
git clone https://github.com/sidequest-chain/antigravity-quota-monitor.git
cd antigravity-quota-monitor

# 2. Compile the optimized, stripped release binary
cargo build --release

# 3. Launch the widget
Start-Process .\target\release\antigravity-quota-monitor.exe
```

The compiled executable (`target\release\antigravity-quota-monitor.exe`) is self-contained (the HTML/CSS/JS frontend is embedded into the binary at compile time by Tauri).

---

## Current Scope and Limitations

- **Windows-Only Credential and Path Resolution:** Credential lookup uses Win32 `CredReadW` (`windows-sys`) and Windows-specific `%LOCALAPPDATA%` / `%APPDATA%` directory conventions. Linux and macOS keychains are not currently supported.
- **Read-Only Session Consumer:** This utility never writes or modifies credentials in Windows Credential Manager or `state.vscdb`. Initial authentication and account switching must be performed inside the official Antigravity CLI (`agy`) or Antigravity IDE.
- **Internal API Dependency:** Quota data is retrieved from Google's `daily-cloudcode-pa.googleapis.com/v1internal:*` endpoints used by the Antigravity client. If Google changes the response schema of those internal endpoints in a future release, the parser in `fetch_usage_internal()` will require an update.
- **Externalized Multi-Language UI (`ui/locales/*.json`):** All user-facing strings (including system tray labels, reset countdown units, and error messages) live exclusively in JSON translation files (`et`, `en-US`, `en-GB`, `lv`, `lt`, `fi`, `de`, `sv`, `pl`, `fr`, `es`, `uk`) with zero hardcoded language strings in Rust, HTML, or JavaScript.

---

## Author, License & Non-Affiliation Disclaimer

- **Author:** [sidequest-chain](https://github.com/sidequest-chain)
- **Repository:** [github.com/sidequest-chain/antigravity-quota-monitor](https://github.com/sidequest-chain/antigravity-quota-monitor)
- **License:** Licensed under the **MIT License** (provided `"AS IS"`, without warranty of any kind) — see [LICENSE](LICENSE).

**Non-Affiliation Disclaimer:** This software is an independent, unofficial open-source companion utility and is **not** affiliated with, endorsed by, sponsored by, or supported by Google LLC or Anthropic PBC. It contains no proprietary keys or credentials and operates exclusively on the user's own local installation.
