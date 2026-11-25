# Antigravity Quota Monitor (`antigravity-quota-monitor`)

`antigravity-quota-monitor` is a native Windows 11 taskbar utility written in Rust and Win32 GDI/DWM (~1.7 MB RAM working set, ~2.5 MB executable, no WebView2 or Chromium processes). It sits on the Windows Taskbar and shows remaining usage quotas, per-model limits, and reset timers for Google Antigravity CLI (`agy`) and Antigravity IDE.

## Why This Exists

Google Antigravity uses rolling usage quotas across its model families: a 5-hour window (`5h`) and a 7-day window (`weekly`), split between Gemini models (Gemini 4 Carbon, Gemini 4 Argon, Gemini 4 Hydrogen, Gemini 3.8 Flash) and third-party models (Claude Opus 5.5, Claude Sonnet 5.5).

Without a dedicated monitor, checking how much quota is left or when a window resets means stopping your work to run `/usage` inside an active CLI session or opening IDE settings. When multiple terminal or editor sessions are open at once, it is easy to hit a rate limit mid-task. This tool places a small two-line indicator on the left side of the Windows 11 Taskbar that switches between 5-hour (`5h`) and weekly (`Wk`) numbers every 10 seconds, and opens a full Flyout panel above the taskbar when you hover over it.

## Read Before Installing

Because this program attaches directly to the Windows 11 Taskbar instead of opening a normal desktop window, check these points first to see if it fits your setup:

1. **It covers the Windows 11 Weather / Temperature ("Widgets") area by default.**
   The `124x38 px` quota pill is placed at the far-left edge of the taskbar (`x = 8 px` scaled for DPI), right on top of the default Windows 11 Weather / Temperature button. If you want to keep the Weather button visible too, hold down the left mouse button on the quota pill and drag it to the right along the taskbar. The new position is saved automatically.
2. **There is no system tray icon and no Alt+Tab window.**
   To avoid adding another icon next to the clock, the program runs only as the taskbar pill and its hover Flyout. To close the program, move your mouse over the pill to open the Flyout and click the power button (`⏻`) in the top-right corner.
3. **The Setup installer enables Windows login auto-start.**
   Running `antigravity-quota-monitor-windows-x64-setup.exe` (or running the binary with `--install`) copies the executable to `%LOCALAPPDATA%\Programs\AntigravityQuotaMonitor\antigravity-quota-monitor.exe` and adds `AntigravityQuotaMonitor` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. If you do not want it starting with Windows, click the bolt icon (`⚡`) in the Flyout header to turn auto-start off, or use the portable `.exe` instead.
4. **It reads your existing local Antigravity session.**
   The program never asks for your Google password and contains no hardcoded Google OAuth keys. It reads the existing token saved on your PC by Antigravity CLI (`gemini:antigravity` in Windows Credential Manager) or Antigravity IDE (`%APPDATA%\Antigravity IDE\User\globalStorage\state.vscdb`), and reads the OAuth client ID from your locally installed `agy.exe` or `language_server_windows_x64.exe` binary when an access token expires.
5. **When not to use this tool:**
   - You do not have Google Antigravity CLI or Antigravity IDE installed and signed in on Windows.
   - You use an auto-hiding taskbar, a top/vertical taskbar, or left-aligned taskbar icons with no free horizontal space on the bar.
   - You want a normal, resizable desktop window rather than a fixed taskbar widget.

## How It Works

On startup, the application checks whether Antigravity CLI or IDE is installed and looks for a local session in three places:
- Windows Credential Manager (`gemini:antigravity`, used by `agy`).
- Antigravity IDE's SQLite database at `%APPDATA%\Antigravity IDE\User\globalStorage\state.vscdb` (opened with `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE` so it works while the IDE is running).
- The ACP token file at `%USERPROFILE%\.gemini\antigravity-acp\acp_token.json`.

When a stored `access_token` expires, `refresh_with_local_oauth_client()` scans the local `agy.exe` or `language_server_windows_x64.exe` binary on disk for `.apps.googleusercontent.com` client IDs and `GOCSPX-` secrets, exchanges the local `refresh_token` against `https://oauth2.googleapis.com/token`, caches the new token in memory, and calls `K32EmptyWorkingSet` so temporary file buffers are freed right away.

Every 5 minutes, a single-threaded background `tokio` worker queries `loadCodeAssist`, `retrieveUserQuotaSummary`, and `fetchAvailableModels` on `https://daily-cloudcode-pa.googleapis.com`, counts active `.lock` files in `%USERPROFILE%\.gemini\antigravity\presence` and `%USERPROFILE%\.gemini\antigravity-cli\presence`, and posts a message (`PostMessageW`) to the Win32 UI thread. Between API polls, a 60-second timer ticks down the displayed reset timers locally.

The UI consists of two native Win32 windows in `src/ui_win32.rs`:
- `AGYPillNativeWnd` (`124x38 px` logical): A top-level `WS_POPUP | WS_EX_TOPMOST | WS_EX_LAYERED` window owned by `Shell_TrayWnd` (`GWLP_HWNDPARENT`) so DWM draws it above the Windows 11 XAML taskbar layer. It samples the taskbar color via `GetPixel` and flips between `5h` and `Wk` views every 10 seconds.
- `AGYFlyoutNativeWnd` (`368x440 px` logical): Opens above the taskbar on hover. It renders side-by-side `5h` and `7d` cards for Gemini and Claude, a scrollable 2-column grid of per-model quotas, header buttons (`Left %` / `Used %` toggle, language menu via `TrackPopupMenu`, Autostart `⚡`, Info drawer `ⓘ`, Refresh `↻`, Quit `⏻`), and a footer with an account switcher, active session count, and window opacity slider.

To switch to Simplified Chinese, open the language menu in the flyout and select `简体中文`.

## Technical Details

- UI and rendering (`src/ui_win32.rs`): Direct Win32 FFI (`user32`, `gdi32`, `dwmapi`) with Per-Monitor DPI Awareness V2, 32-bit top-down `DIBSection` software SDF anti-aliased shapes (`fill_round_rect`, `fill_round_rect_gradient`, `stroke_round_rect`, `fill_circle`, `draw_line_aa`), and ClearType `DrawTextW` text using `Segoe UI`.
- Networking and credentials (`src/main.rs`): Rust (2021 edition) with a single-threaded `tokio` runtime, `reqwest` v0.12 (`rustls-tls`, no OpenSSL), and direct `advapi32` / `kernel32` FFI bindings (`CredReadW`, `CredFree`, `RegOpenKeyExW`, `RegSetValueExW`, `K32EmptyWorkingSet`).

```text
.
├── Cargo.toml                 # Crate manifest, dependencies, and release LTO/strip profile
├── installer.nsi              # NSIS Windows Setup installer script (per-user install + autostart)
├── icons/
│   └── icon.ico               # Windows installer and shortcut icon
├── src/
│   ├── main.rs                # Credential readers, binary OAuth scanner, API client, taskbar geometry, and autostart
│   └── ui_win32.rs            # Win32 window classes, SDF 2D canvas, ClearType text, and Flyout UI
└── locales/                   # JSON language packs (ET, EN-US, EN-GB, ZH-CN, LV, LT, FI, DE, SV, PL, FR, ES, UK)
```

## Installation and Removal

### Prerequisites

- Windows 10 or Windows 11 (x64). No WebView2 or external runtime is needed.
- Google Antigravity CLI (`%LOCALAPPDATA%\agy\bin\agy.exe`) or Antigravity IDE (`%LOCALAPPDATA%\Programs\Antigravity-IDE\Antigravity IDE.exe`) installed and signed in.

### Option 1: Windows Setup Installer

Download `antigravity-quota-monitor-windows-x64-setup.exe` from [GitHub Releases](https://github.com/sidequest-chain/antigravity-quota-monitor/releases) and run it. It does not require administrator rights. It installs the binary to `%LOCALAPPDATA%\Programs\AntigravityQuotaMonitor`, creates a Start Menu shortcut and Windows *Installed Apps* uninstaller, enables Windows login auto-start, and starts the widget.

### Option 2: Portable Binary

Download `antigravity-quota-monitor-windows-x64.exe` and run it from any folder. You can also install or uninstall it from PowerShell:

```powershell
# Copy to %LOCALAPPDATA%\Programs\AntigravityQuotaMonitor, create Start Menu shortcut, enable auto-start, and launch
.\antigravity-quota-monitor.exe --install

# Remove auto-start registry entry and Start Menu shortcut
.\antigravity-quota-monitor.exe --uninstall
```

### How to Quit or Uninstall

- Quit the running widget: Hover over the taskbar pill and click `⏻` in the top-right corner of the Flyout.
- Turn off Windows login auto-start: Hover over the taskbar pill and click `⚡` in the Flyout header so it turns grey.
- Uninstall completely: Open Windows *Settings -> Apps -> Installed apps*, find **Antigravity Quota Monitor**, and click **Uninstall** (or run `%LOCALAPPDATA%\Programs\AntigravityQuotaMonitor\Uninstall.exe`). Saved window position and preferences live in `%USERPROFILE%\.gemini\antigravity-cli\window_state.json`.

### Building from Source

Requires Rust (`rustup` + `cargo`) with the `x86_64-pc-windows-msvc` toolchain and Visual Studio C++ Build Tools:

```powershell
git clone https://github.com/sidequest-chain/antigravity-quota-monitor.git
cd antigravity-quota-monitor
cargo build --release
.\target\release\antigravity-quota-monitor.exe
```

## Scope and Limitations

- Windows only: Uses Win32 `CredReadW` and `Shell_TrayWnd`. Linux and macOS are not supported.
- Read-only session access: Never writes to Windows Credential Manager or `state.vscdb`. Initial sign-in and account switching happen in the official Antigravity CLI or IDE.
- Internal API endpoints: Reads quota data from `daily-cloudcode-pa.googleapis.com/v1internal:*`. If Google changes those response payloads in a future update, `fetch_usage_internal()` in `src/main.rs` will need updating.

## License and Disclaimer

- Author: [sidequest-chain](https://github.com/sidequest-chain)
- Repository: [github.com/sidequest-chain/antigravity-quota-monitor](https://github.com/sidequest-chain/antigravity-quota-monitor)
- License: MIT License (provided `"AS IS"`, without warranty of any kind). See [LICENSE](LICENSE).

This project is an independent open-source utility and is not affiliated with, endorsed by, or supported by Google LLC or Anthropic PBC.
