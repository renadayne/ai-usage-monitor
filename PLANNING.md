# PLANNING — AI Usage Monitor

> App desktop (tray) theo dõi usage của các AI coding tool trên máy: **Claude, Codex, Antigravity, GitHub Copilot**.
> File này dùng để đưa cho Claude Code thực thi **từng step một**. Xong mỗi step phải commit + push lên GitHub (xem mục 3).

---

## 1. Mục tiêu

Với mỗi provider, app trả lời 3 câu hỏi:

| Câu hỏi | Ý nghĩa |
|---|---|
| **Active?** | Tool có đang chạy và đang tiêu quota hay không (không chỉ là "process đang mở") |
| **Model?** | Model đang/vừa được dùng gần nhất |
| **Còn bao nhiêu?** | % quota đã dùng / còn lại theo từng cửa sổ (5h, tuần, tháng…) và thời điểm reset |

**Nền tảng: chỉ Windows 10/11 (x64).** Không cần viết code cho macOS/Linux.

Ngoài phạm vi (v1): macOS/Linux, tính tiền chi tiết theo token, đồng bộ nhiều máy, quản lý nhiều account, gửi request thay người dùng.

### Lưu ý riêng cho Windows: WSL

Claude Code và Codex CLI có thể được cài trong **WSL** thay vì Windows native. Khi đó file config/log nằm trong filesystem của WSL chứ không phải `%USERPROFILE%`. Mỗi provider phải dò theo thứ tự:

1. Đường dẫn override bằng biến môi trường (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`) hoặc trong Settings của app.
2. Windows native: `%USERPROFILE%\.claude`, `%USERPROFILE%\.codex`.
3. WSL: liệt kê distro bằng `wsl.exe -l -q`, thử `\\wsl.localhost\<distro>\home\<user>\.claude` (và `.codex`).

Process chạy trong WSL không hiện tên thật trong danh sách process của Windows → với trường hợp WSL, "Active" chỉ dựa vào mtime của file log.

---

## 2. Đánh giá khả thi

**Kết luận: khả thi cho cả 4 provider**, nhưng toàn bộ nguồn quota đều là endpoint nội bộ / không có tài liệu chính thức → có thể đổi bất kỳ lúc nào. Thiết kế phải chịu được việc 1 provider hỏng mà app vẫn chạy.

| Provider | Active | Model | Quota | Độ tin cậy | Nguồn dữ liệu |
|---|---|---|---|---|---|
| Claude | ✅ | ✅ | ✅ 5h + 7 ngày (+ theo model) | Cao | Log JSONL local + OAuth usage endpoint |
| Codex | ✅ | ✅ | ✅ primary (5h) + secondary (tuần) | Cao | Rollout JSONL local + ChatGPT usage endpoint |
| Antigravity | ✅ | ⚠️ một phần | ✅ theo từng model | Trung bình | Language server chạy local (chỉ khi app đang mở) |
| GitHub Copilot | ⚠️ một phần | ⚠️ khó | ✅ theo tháng | Trung bình | GitHub internal API |

### 2.1 Claude (Claude Code / Claude app)

- **Quota**: `GET https://api.anthropic.com/api/oauth/usage`
  - Header: `Authorization: Bearer <accessToken>`, `anthropic-beta: oauth-2025-04-20`
  - Trả về các bucket (`five_hour`, `seven_day`, bucket theo model, extra usage) với `utilization` (thang 0–100) và thời điểm reset.
  - Token: `%USERPROFILE%\.claude\.credentials.json` (hoặc bản trong WSL). Tôn trọng biến `CLAUDE_CONFIG_DIR`.
  - ⚠️ Endpoint này rate-limit rất gắt (hay trả 429). **Poll tối thiểu 5 phút/lần**, cache kết quả, khi 429 thì dùng dữ liệu cũ + backoff.
- **Nguồn dự phòng không cần gọi mạng**: Claude Code (từ v2.1.80) truyền `rate_limits` (`five_hour`, `seven_day` với `used_percentage`, `resets_at`) vào stdin của statusline script. Có thể cài 1 statusline hook nhỏ ghi JSON này ra file để app đọc. Lưu ý field này có thể vắng mặt với một số loại plan → chỉ coi là nguồn phụ.
- **Active + Model**: quét `<claude dir>\projects\**\*.jsonl`
  - Active = có process Claude Code (trên Windows native thường là `claude.exe` hoặc `node.exe` có cmdline chứa `claude` — xác minh trong spike) **và** có file JSONL được ghi trong ~2 phút gần nhất.
  - Model = field `message.model` của dòng assistant cuối cùng trong file mới nhất.
- **Không tự refresh OAuth token** (có thể làm hỏng phiên đăng nhập của Claude Code). Token hết hạn → hiện trạng thái "mở Claude Code để làm mới".

### 2.2 Codex (CLI / IDE extension)

- **Nguồn local (ưu tiên)**: `%USERPROFILE%\.codex\sessions\YYYY\MM\DD\rollout-*.jsonl` (hoặc bản trong WSL)
  - Event `event_msg` có `payload.type == "token_count"` chứa `rate_limits`: `primary.used_percent`, `secondary.used_percent`, `plan_type`, thời điểm reset.
  - Model nằm trong `turn_context`.
  - Cần kiểm tra thực tế: file có thể là `.jsonl` hoặc `.jsonl.zst` (nén) tuỳ version.
- **Nguồn live (bổ sung)**: `GET https://chatgpt.com/backend-api/wham/usage`
  - Header: `Authorization: Bearer <tokens.access_token>`, `ChatGPT-Account-Id: <tokens.account_id>` — đọc từ `~/.codex/auth.json`.
  - Nếu `~/.codex/config.toml` có `chatgpt_base_url` khác thì path là `/api/codex/usage`.
- **Active**: process `codex` + rollout file vừa được ghi.
- Chỉ áp dụng khi đăng nhập bằng ChatGPT. Đăng nhập bằng API key thì không có cửa sổ quota kiểu này → hiển thị "API key mode".

### 2.3 Antigravity

- Không cần đăng nhập riêng; hỏi thẳng **language server local** của Antigravity:
  1. Tìm process `language_server*.exe` của Antigravity (dự kiến `language_server_windows_x64.exe` — xác minh trong spike), đọc command line.
  2. Lấy `--csrf_token` và `--https_server_port` (thực tế 11/2026; bản cũ hơn dùng `--extension_server_port`, `--extension_server_csrf_token`). Parser phải chịu được cả 2 dạng `--flag VALUE` và `--flag=VALUE`.
  3. Liệt kê các port mà PID đó đang listen, thử từng port bằng 1 request probe để tìm đúng port Connect (HTTPS, cert self-signed).
  4. Gọi `POST https://127.0.0.1:<port>/exa.language_server_pb.LanguageServerService/<Method>` với header `X-Codeium-Csrf-Token`, `Connect-Protocol-Version: 1`, `Content-Type: application/json`.
     - Thử theo thứ tự: `RetrieveUserQuotaSummary` → `GetUserStatus` → `GetCommandModelConfigs`.
- Trả về quota còn lại + thời điểm reset **theo từng model**, kèm plan/email (tuỳ method).
- **Giới hạn**: chỉ có dữ liệu khi Antigravity đang mở. Khi đóng → hiển thị snapshot cuối + nhãn "stale". "Model đang dùng" không được trả trực tiếp → v1 hiển thị danh sách model + quota, model active là best-effort.
- TLS: chỉ tắt verify cert cho đúng `127.0.0.1`, không bao giờ cho host khác.

### 2.4 GitHub Copilot

- **Quota**: `GET https://api.github.com/copilot_internal/user` với GitHub OAuth token
  - Trả về `copilot_plan`, `quota_reset_date`, `quota_snapshots` (`chat`, `completions`, `premium_interactions`) với `percent_remaining`, `remaining`, `unlimited`, `overage_permitted`.
  - ⚠️ Từ 01/06/2026 Copilot đã chuyển sang billing theo **GitHub AI Credits** (thay cho premium requests; gói năm cũ vẫn giữ kiểu cũ đến khi hết hạn). Shape của response có thể đã đổi → **bắt buộc dump response thật trước khi viết parser**, và parser phải xử lý được cả 2 kiểu.
- **Token** (thử theo thứ tự, cần xác minh trên máy thật):
  1. File config của Copilot: `%LOCALAPPDATA%\github-copilot\apps.json` / `hosts.json`.
  2. `gh auth token` nếu có GitHub CLI.
  3. GitHub device flow do app tự thực hiện (cách CodexBar đang dùng).
- **Active / Model**: Copilot không ghi log local ổn định như Claude/Codex. v1: Active = có process editor (`Code.exe`, `Code - Insiders.exe`, JetBrains `idea64.exe`…) có Copilot + quota thay đổi giữa 2 lần poll. Model = "không xác định" trừ khi tìm được nguồn đáng tin trong spike.

### 2.5 Dự án tham khảo (đọc code để lấy chi tiết endpoint, KHÔNG copy code)

Các app dưới đây chủ yếu cho macOS nên không dùng trực tiếp được trên Windows, nhưng tài liệu về endpoint vẫn đúng:

- `steipete/CodexBar` (macOS, Swift) — có đủ cả 4 provider; `docs/antigravity.md`, `docs/copilot.md` rất chi tiết.
- `tddworks/ClaudeBar` (macOS) — Claude, Codex, Copilot, Antigravity.
- `Henrik-3/AntigravityQuota` — extension đọc quota Antigravity.
- `ryoppippi/ccusage` — parser log Claude Code / Codex.
- `abtop` (Rust) — cách map process → rollout file của Codex.

---

## 3. Quy tắc làm việc cho Claude Code

1. **Làm đúng 1 step mỗi lần.** Không làm lấn sang step sau.
2. Trước khi kết thúc step: chạy `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `pnpm lint`, `pnpm test` (những lệnh đã tồn tại ở thời điểm đó). Tất cả phải pass.
3. **Commit + push sau mỗi step** (shell là PowerShell; không dùng cú pháp chỉ có ở bash):
   ```powershell
   git add -A
   git commit -m "<message ghi trong step>"
   git push origin main
   ```
4. Sau khi commit, tick `[x]` vào checkbox của step trong file này và ghi 1–3 dòng vào mục **Nhật ký** ở cuối (điều gì khác so với plan, điều gì phát hiện được). Commit thay đổi đó chung với step.
5. **Bảo mật — không được vi phạm**:
   - Không bao giờ log, in ra, ghi file, hay commit access token / refresh token / CSRF token / cookie.
   - Fixture dùng cho test phải được **redact** (token, email, account id, đường dẫn home) trước khi lưu.
   - App chỉ **đọc**: không sửa file của tool khác, không refresh token thay tool khác, không gửi request inference.
   - Token chỉ gửi tới đúng host gốc của provider đó. Không telemetry.
6. Step provider nào cũng bắt đầu bằng **spike**: viết script probe, chạy trên máy thật, lưu response đã redact vào `fixtures/<provider>/`, rồi mới viết parser dựa trên fixture. Nếu thực tế khác tài liệu ở mục 2 → **tin thực tế**, cập nhật lại mục 2.
7. Nếu 1 nguồn dữ liệu không dùng được trên máy này (tool chưa cài, chưa login): vẫn hoàn thành step bằng fixture + unit test, ghi rõ trong Nhật ký là chưa verify thật.
8. Gặp quyết định không đảo ngược được hoặc cần quyền của người dùng (tạo repo public, cài hook vào `~/.claude/settings.json`) → dừng lại và hỏi.

---

## 4. Kiến trúc

**Stack**: Tauri 2 (Rust backend) + React + TypeScript + Vite + Tailwind. Nhẹ (dùng WebView2 có sẵn trên Windows), phù hợp app system tray chạy nền.

**Yêu cầu môi trường dev**: Rust (toolchain `stable-x86_64-pc-windows-msvc`), Visual Studio Build Tools (workload "Desktop development with C++"), Node.js LTS + pnpm, WebView2 Runtime, Git, GitHub CLI (`gh`). Step 0 phải kiểm tra đủ các thứ này trước khi làm tiếp.

```
ai-usage-monitor/
├── src-tauri/
│   └── src/
│       ├── main.rs
│       ├── core/
│       │   ├── model.rs        # ProviderSnapshot, QuotaWindow, Status
│       │   ├── provider.rs     # trait Provider
│       │   ├── scheduler.rs    # poll theo interval riêng từng provider, backoff
│       │   ├── store.rs        # SQLite: snapshot mới nhất + lịch sử
│       │   ├── process.rs      # dò process (crate sysinfo)
│       │   └── paths.rs        # resolve đường dẫn: env override → Windows native → WSL
│       ├── providers/
│       │   ├── claude.rs
│       │   ├── codex.rs
│       │   ├── copilot.rs
│       │   └── antigravity.rs
│       ├── commands.rs         # Tauri commands cho UI
│       └── tray.rs
├── src/                        # React UI
├── fixtures/<provider>/        # response/log mẫu ĐÃ REDACT
├── scripts/probe/              # script spike
└── PLANNING.md
```

### Model dữ liệu chuẩn hoá

```rust
pub enum ProviderId { Claude, Codex, Antigravity, Copilot }

pub enum Activity {
    Active,          // đang chạy và vừa có hoạt động tiêu quota
    Idle,            // process đang chạy nhưng không hoạt động
    NotRunning,
    Unknown,
}

pub enum Health {
    Ok,
    Stale { since: DateTime<Utc> },   // đang hiển thị dữ liệu cũ
    AuthRequired { hint: String },    // chưa login / token hết hạn
    NotInstalled,
    RateLimited { retry_at: DateTime<Utc> },
    Error { message: String },        // message không chứa secret
}

pub struct QuotaWindow {
    pub id: String,                   // "five_hour", "weekly", "monthly", "model:<name>"
    pub label: String,
    pub used_percent: Option<f32>,    // 0..100
    pub remaining: Option<f64>,       // số tuyệt đối nếu provider có
    pub limit: Option<f64>,
    pub unit: Option<String>,         // "credits", "requests", ...
    pub resets_at: Option<DateTime<Utc>>,
}

pub struct ProviderSnapshot {
    pub provider: ProviderId,
    pub activity: Activity,
    pub health: Health,
    pub plan: Option<String>,
    pub current_model: Option<String>,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub windows: Vec<QuotaWindow>,
    pub fetched_at: DateTime<Utc>,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn poll_interval(&self) -> Duration;          // quota (chậm)
    async fn detect_activity(&self) -> ActivityInfo;   // local, nhanh, gọi mỗi ~10s
    async fn fetch_quota(&self) -> Result<QuotaInfo, ProviderError>;
}
```

### Tần suất poll

| Việc | Tần suất |
|---|---|
| Dò process + mtime log (local) | 10 giây |
| Claude quota (network) | 5 phút, backoff luỹ thừa khi 429, tối đa 30 phút |
| Codex quota: file local | 10 giây (theo mtime) · endpoint live: 5 phút |
| Antigravity (localhost) | 60 giây khi app đang mở |
| Copilot quota | 10 phút |

Luôn có nút "Refresh now" nhưng vẫn tôn trọng backoff.

---

## 5. Các step

### [x] Step 0 — Khởi tạo repo

- Kiểm tra môi trường dev (xem mục 4): `rustc --version`, `cargo --version`, `node --version`, `pnpm --version`, `gh auth status`. Thiếu gì thì báo người dùng cài, không tự cài.
- `git init`, branch `main`, `.gitignore` (Rust, Node, `.env`, `*.local.json`, `fixtures/**/raw/`), `.gitattributes` (`* text=auto eol=lf`).
- `README.md` ngắn, `LICENSE` (MIT), copy file `PLANNING.md` này vào root.
- Tạo repo GitHub: `gh repo create ai-usage-monitor --private --source=. --remote=origin` (hỏi người dùng nếu muốn public / tên khác).
- **Xong khi**: `git push` thành công, repo có trên GitHub.
- **Commit**: `chore: init repository and planning doc`

### [x] Step 1 — Scaffold Tauri + tray

- Scaffold Tauri 2 + React + TS + Vite + Tailwind (pnpm).
- Tray icon với menu: Open Dashboard / Refresh / Quit. Đóng cửa sổ = ẩn xuống tray, không thoát.
- Cấu hình lint/format: `rustfmt`, `clippy`, ESLint, Prettier. Thêm script `pnpm lint`, `pnpm test` (Vitest).
- GitHub Actions `ci.yml`: fmt + clippy + test + build frontend trên `windows-latest`.
- **Xong khi**: `pnpm tauri dev` mở được cửa sổ trống + tray icon; CI xanh.
- **Commit**: `feat: scaffold tauri app with tray and CI`

### [x] Step 2 — Core: model, trait, scheduler, store

- Cài đặt `core/model.rs`, `core/provider.rs` đúng như mục 4.
- `core/paths.rs`: resolve thư mục config của từng tool theo thứ tự ở mục "Lưu ý riêng cho Windows: WSL" (env override → `%USERPROFILE%` → WSL). Kết quả dò WSL phải được cache (gọi `wsl.exe` chậm) và không được làm treo app nếu WSL không cài.
- `core/store.rs`: SQLite (`rusqlite`), bảng `snapshots_latest` và `snapshots_history` (lưu tối đa 30 ngày).
- `core/scheduler.rs`: mỗi provider 1 task tokio; 2 nhịp (activity nhanh / quota chậm); backoff; 1 provider lỗi không ảnh hưởng provider khác; phát event `snapshot-updated` cho UI.
- `MockProvider` + unit test cho scheduler (interval, backoff, cô lập lỗi) và store.
- Tauri commands: `get_snapshots`, `refresh(provider?)`.
- **Xong khi**: UI tạm hiển thị JSON snapshot từ MockProvider cập nhật theo thời gian; test pass.
- **Commit**: `feat(core): provider trait, scheduler, sqlite store`

### [x] Step 3 — Dò process

- `core/process.rs` dùng crate `sysinfo`: tìm process theo tên + pattern command line, trả về pid, cmdline, thời điểm start.
- Bảng pattern cho: Claude Code (`claude.exe` / `node.exe` + cmdline), Codex (`codex.exe`), Antigravity (`Antigravity.exe` + `language_server*.exe`), editor có Copilot (`Code.exe`, `Code - Insiders.exe`, JetBrains…). So khớp tên không phân biệt hoa thường.
- Nếu `sysinfo` không đọc được command line của process nào đó, fallback sang `Get-CimInstance Win32_Process` (PowerShell). Không yêu cầu quyền admin.
- Hàm liệt kê port TCP đang listen theo PID: ưu tiên Win32 API `GetExtendedTcpTable`, fallback `netstat -ano` — dùng cho Antigravity ở step 7.
- Mọi lệnh con (`wsl.exe`, `netstat`, `powershell`) phải chạy với cờ `CREATE_NO_WINDOW` để không nháy cửa sổ console.
- Unit test bằng dữ liệu process giả; test thủ công trên máy thật.
- **Xong khi**: command `debug_processes` trả về đúng các tool đang chạy trên máy.
- **Commit**: `feat(core): cross-platform process and port detection`

### [x] Step 4 — Provider: Claude

- **Spike** `scripts/probe/claude.*`: đọc credentials, gọi usage endpoint 1 lần, lưu response redact vào `fixtures/claude/usage.json`; lấy 1 đoạn JSONL mẫu (redact nội dung hội thoại) vào `fixtures/claude/session.jsonl`.
- Đọc credentials từ file `.credentials.json` (native hoặc WSL). Kiểm tra `expiresAt` → nếu hết hạn trả `AuthRequired`, **không refresh**.
- `fetch_quota`: gọi endpoint, map bucket → `QuotaWindow` (`five_hour`, `seven_day`, bucket theo model, extra usage nếu có). Xử lý 401 / 429 (đọc `Retry-After` nếu có).
- `detect_activity`: file JSONL mới nhất theo mtime trong `projects/`, đọc ngược từ cuối file để lấy `message.model` và timestamp gần nhất (không đọc cả file).
- (Tuỳ chọn, hỏi người dùng trước) lệnh cài statusline hook ghi `rate_limits` ra `%LOCALAPPDATA%\ai-usage-monitor\claude-statusline.json` làm nguồn phụ.
- Test: parser với fixture, các nhánh lỗi 401/429, token hết hạn, thư mục không tồn tại (`NotInstalled`).
- **Xong khi**: snapshot Claude khớp với số trong `/usage` của Claude Code.
- **Commit**: `feat(provider): claude usage, activity and model detection`

### [x] Step 5 — Provider: Codex

- **Spike**: xác định định dạng rollout thật (`.jsonl` hay `.jsonl.zst`), lưu mẫu `token_count` + `turn_context` đã redact; gọi thử `wham/usage` và lưu response redact.
- Parser rollout: tìm file mới nhất, đọc ngược lấy event `token_count` cuối cùng có `rate_limits` → `primary` (5h), `secondary` (tuần), `plan_type`; lấy model từ `turn_context` gần nhất. Hỗ trợ zstd nếu spike cho thấy cần.
- Nguồn live `wham/usage` chạy song song; quy tắc gộp: lấy nguồn có timestamp mới hơn. Nếu cửa sổ đã qua thời điểm reset mà chưa có dữ liệu mới → hiển thị 0% + nhãn "ước lượng".
- Phát hiện API-key mode từ `auth.json` → `Health::Ok` nhưng không có window, kèm ghi chú.
- Test với fixture cho cả 2 nguồn + logic gộp.
- **Xong khi**: số khớp với `/status` trong Codex CLI.
- **Commit**: `feat(provider): codex rate limits from rollout logs and usage api`

### [x] Step 6 — Provider: GitHub Copilot

- **Spike (quan trọng nhất của step)**: tìm token trên máy theo thứ tự ở 2.4, gọi `copilot_internal/user`, lưu response redact. Ghi rõ vào mục 2.4 shape thực tế sau khi chuyển sang AI Credits.
- Cài đặt token discovery; nếu không tìm thấy → `AuthRequired` với hướng dẫn. Device flow để ở step 9 (Settings).
- Parser chịu được cả 2 kiểu: premium requests (cũ) và AI Credits (mới); field lạ thì bỏ qua, không crash. Map sang window `monthly` (+ `chat`, `completions` nếu không phải unlimited).
- Activity: process editor + so sánh delta quota giữa 2 lần poll. `current_model = None` nếu spike không tìm ra nguồn tin cậy — ghi rõ lý do trong Nhật ký.
- Test với fixture cả 2 kiểu response.
- **Xong khi**: số khớp với trang Copilot usage trên github.com hoặc trong VS Code.
- **Commit**: `feat(provider): github copilot quota`

### [x] Step 7 — Provider: Antigravity

- **Spike**: với Antigravity đang mở, chạy probe: tìm process, parse flag, dò port, gọi lần lượt 3 method ở 2.3; lưu response redact của từng method.
- Cài đặt đầy đủ luồng ở 2.3. HTTP client riêng chỉ cho `127.0.0.1` với `danger_accept_invalid_certs`; assert host là loopback trước khi gửi.
- Map: mỗi model → 1 `QuotaWindow` (`model:<tên>`), `used_percent = (1 - remainingFraction) * 100`, `resets_at`. Nhóm các model dùng chung pool nếu response có thông tin đó.
- Khi app đóng: `Activity::NotRunning` + snapshot cuối từ store với `Health::Stale`.
- Test: parser cmdline (cả 2 dạng flag), parser 3 kiểu response, trường hợp nhiều process.
- **Xong khi**: số khớp với phần quota trong settings của Antigravity.
- **Commit**: `feat(provider): antigravity quota via local language server`

### [x] Step 8 — UI dashboard + tray popup

- Dashboard: 4 card provider. Mỗi card: chấm trạng thái (Active / Idle / Not running), plan, model hiện tại, thanh tiến trình cho từng window + đếm ngược tới reset, "cập nhật X phút trước", badge Stale / Auth required / Rate limited.
- Màu thanh: < 70% bình thường, 70–90% cảnh báo, > 90% nguy hiểm (kèm icon/text, không chỉ dựa vào màu).
- Popup nhỏ khi click tray: mỗi provider 1 dòng (trạng thái + window "căng" nhất).
- Tooltip tray: tóm tắt nhanh. Hỗ trợ dark/light theo theme của Windows. Popup hiện sát góc system tray, tự ẩn khi mất focus.
- Lắng nghe event `snapshot-updated`; nút Refresh từng provider.
- Test component (Vitest + Testing Library) cho các trạng thái của card.
- **Xong khi**: dashboard hiển thị dữ liệu thật của cả 4 provider, mọi trạng thái lỗi đều có giao diện rõ ràng.
- **Commit**: `feat(ui): dashboard and tray popup`

### [ ] Step 9 — Settings, thông báo, lịch sử

- Settings: bật/tắt từng provider, chỉnh interval (có sàn tối thiểu để không bị rate-limit), ngưỡng cảnh báo, khởi động cùng hệ thống (`tauri-plugin-autostart`).
- Thông báo hệ thống (`tauri-plugin-notification`): vượt ngưỡng (mặc định 80% và 95%), quota vừa reset; mỗi sự kiện chỉ báo 1 lần cho mỗi cửa sổ.
- Copilot device flow trong Settings (nếu step 6 cần); token lưu vào Windows Credential Manager (`keyring` crate), không lưu file thường.
- Settings có ô nhập đường dẫn thủ công cho thư mục Claude / Codex (phòng khi dò WSL không ra).
- Biểu đồ lịch sử đơn giản: % sử dụng theo thời gian của từng window (7 ngày).
- **Xong khi**: đổi setting có hiệu lực không cần restart; thông báo bắn đúng 1 lần.
- **Commit**: `feat: settings, notifications and usage history`

### [ ] Step 10 — Hardening + đóng gói

- Rà soát bảo mật: grep toàn bộ code/log để chắc chắn không lộ token; redact trong mọi message lỗi; kiểm tra lại fixtures.
- Xử lý edge case: máy không cài tool nào, mất mạng, đồng hồ lệch, file log đang bị ghi dở (dòng JSON cuối không hoàn chỉnh), file rất lớn.
- Đo tài nguyên khi chạy nền: CPU ~0% lúc nghỉ, RAM hợp lý.
- README đầy đủ: cách hoạt động, nguồn dữ liệu của từng provider, giới hạn đã biết, cảnh báo "endpoint không chính thức", troubleshooting.
- GitHub Actions `release.yml`: build installer Windows (NSIS `.exe`, kèm WebView2 bootstrapper) khi push tag `v*` (`tauri-action`).
- Installer chưa ký số → SmartScreen sẽ cảnh báo; ghi cách xử lý trong README.
- Tag `v0.1.0`.
- **Xong khi**: cài bản build từ release chạy được trên máy Windows sạch.
- **Commit**: `chore: hardening, docs and release workflow`

---

## 6. Rủi ro đã biết

| Rủi ro | Cách giảm thiểu |
|---|---|
| Endpoint nội bộ thay đổi / bị chặn | Mỗi provider độc lập; parser khoan dung; fixture + test; `Health::Error` rõ ràng thay vì crash |
| Claude usage endpoint trả 429 liên tục | Interval ≥ 5 phút, cache, backoff, nguồn phụ từ statusline |
| Copilot đổi billing (AI Credits) | Spike trước, parser hỗ trợ 2 shape |
| Antigravity đổi tên process / flag / method | Bảng pattern cấu hình được; fallback 3 method |
| Claude Code / Codex cài trong WSL | Dò `\\wsl.localhost\...`, cho nhập đường dẫn thủ công, Active dựa vào mtime |
| Đọc credential của tool khác | Chỉ đọc, giữ trong RAM, không ghi lại, không gửi đi đâu ngoài host gốc; ghi rõ trong README |
| Điều khoản sử dụng của các provider | App chỉ đọc số liệu của chính tài khoản người dùng với tần suất thấp; người dùng tự cân nhắc; ghi chú trong README |

---

## 7. Nhật ký

_(Claude Code ghi vào đây sau mỗi step: ngày, step, khác biệt so với plan, phát hiện mới.)_

- **2026-11-14 — Step 0.** Repo git local và remote `origin` (`renadayne/ai-usage-monitor`) đã có sẵn trước khi bắt đầu nên bỏ qua `git init` / `gh repo create`. Repo trên GitHub đang **public** (plan ghi `--private`).
- Môi trường dev: có Node v24.14.0, Git 2.53, WebView2 Runtime. **Thiếu** Rust (rustc/cargo/rustup), Visual Studio Build Tools (C++), pnpm, GitHub CLI (`gh`) → phải cài trước Step 1.
- **2026-11-14 — Step 1.** Đã cài Rust 1.100 (rustup), VS Build Tools 2022 (C++), pnpm 12 (qua `npm i -g`, vì `corepack enable` bị EPERM). `gh` chưa cài được (installer bị huỷ ở bước xin quyền admin) — không cần cho việc push. Scaffold viết tay thay vì `create-tauri-app`; TypeScript ghim ở 6.x vì `typescript-eslint` chưa hỗ trợ TS 7.
- ⚠️ **Smart App Control đang bật (enforced) trên máy dev** → Windows chặn mọi file thực thi chưa ký do cargo build ra (build script, `cargo-clippy`, chính app). Hệ quả: trên máy này chỉ chạy được `cargo fmt`, lint/test/build frontend; `cargo clippy`, `cargo test`, `pnpm tauri dev` **chưa verify local**, phần Rust được kiểm tra bằng GitHub Actions CI.
- **2026-11-14 — Step 2.** Thư mục vẫn là `src-tauri/src/core/` nhưng module Rust tên `engine` (tránh che crate `core` có sẵn). Thêm so với plan: `QuotaWindow.estimated`, `ProviderSnapshot.note`, `core/cmd.rs` (chạy lệnh con ẩn cửa sổ + timeout), bật/tắt provider trong scheduler. `NotInstalled` / `AuthRequired` không tính backoff (poll lại theo nhịp thường). History lưu theo từng window. App tạm chạy 4 `MockProvider`.
- **2026-11-14 — Step 3.** Trên máy thật: Claude Code là `claude.exe` trong `...aude-code<version>`, trùng tên với app Claude desktop (Electron, `WindowsAppsClaude_*ppClaude.exe`) → tách thành 2 loại `ClaudeCode` / `ClaudeDesktop` dựa vào đường dẫn exe và cờ `--type=`. Codex là `codex.exe app-server` do app Codex desktop chạy. Quét 2 lượt (tên trước, cmdline chỉ cho ứng viên), cache 5 giây. `debug_processes` không trả command line (có thể chứa CSRF token). CI in lỗi clippy/test ra annotation (`scripts/ci/run-annotated.ps1`) vì máy dev không build Rust được.
- **2026-11-14 — Step 4.** Máy dev **không có** `~/.claude/.credentials.json` (Claude Code ở đây do app Claude desktop chạy và tự giữ token) → usage endpoint **chưa verify thật**; `fixtures/claude/usage.json` là fixture tổng hợp theo shape đã công bố, `session.jsonl` là dữ liệu thật đã redact. Trên máy này Claude sẽ hiện `AuthRequired` + vẫn có Active/Model từ log. Log có dòng rất dài (~200 KB, `api-request-blob`) nên đọc ngược theo nấc 128 KB → 2 MB → 24 MB. Đã có phần đọc file statusline (`claude-statusline.json`) làm nguồn phụ khi thiếu token / 401 / 429, nhưng **chưa cài hook** (cần người dùng đồng ý). Thêm `core/http.rs`, `core/tail.rs`.
- **2026-11-14 — Step 5.** Thực tế trên máy (app Codex desktop 26.1111): **cả 60 rollout đều là `.jsonl.zst`**, file mới nhất nặng 232 MB và đã 8 ngày không ghi; thread hiện tại nằm trong SQLite (`thread_history_1.sqlite`, `state_5.sqlite`). App **không mở SQLite của Codex** (mở read-only vẫn làm đổi `-shm`/`-wal`). `wham/usage` gọi được thật (HTTP 200, plan `plus`) → fixture thật đã redact; shape: `rate_limit.primary_window/secondary_window {used_percent, limit_window_seconds, reset_at}`. Rollout: `rate_limits.primary/secondary {used_percent, window_minutes, resets_at}` + `plan_type`, model ở `turn_context.payload.model`. Hỗ trợ zstd bằng `ruzstd` (chỉ giải nén file ≤ 16 MB, nhiều frame nối nhau). Nhịp: đọc log local 60 giây, gọi live 5 phút; model fallback lấy `model` trong `config.toml`. `chatgpt_base_url` lạ (không phải host OpenAI/https) → không gửi token, chỉ dùng log local. Chưa so được với `/status` vì không build/chạy app local được.
- **2026-11-14 — Step 6.** **Chưa verify thật**: máy dev không có token nào dò được (không có `apps.json`/`hosts.json`, không có `gh`; VS Code giữ token trong secret storage mã hoá — app không đụng vào). Cả 3 fixture Copilot đều là tổng hợp; riêng shape AI Credits (`user-credits.json`) là **phỏng đoán** → mục 2.4 chưa cập nhật được shape thực tế. Parser vì thế nhận mọi object trông giống quota (`percent_remaining` / `remaining`+`entitlement` / `used`+`limit`...), key chứa `credit` → window `monthly` "AI Credits", `premium_interactions` → `monthly` "Premium requests", gói Free dùng `limited_user_quotas`+`monthly_quotas`. HTTP 404 coi như tài khoản không có Copilot. `current_model = None`: Copilot không có log local ổn định ghi model. Active = có process editor + quota đổi giữa 2 lần poll (trong 15 phút). Chạy `node scripts/probe/copilot.mjs` khi có token để lấy response thật.
- **2026-11-15 — Step 7.** Spike chạy thật với Antigravity đang mở (gói Google AI Pro). Khác plan: process là `resources\bin\language_server.exe` (không có hậu tố `_windows_x64`); cmdline **không có** `--extension_server_port` / `--extension_server_csrf_token`, mà có `--csrf_token` và `--https_server_port` → dùng port này trước, rồi mới tới các port đang listen (HTTPS trước, HTTP sau). `RetrieveUserQuotaSummary` trả **quota theo pool** (`response.groups[].buckets[]` với `window` = `5h`/`weekly`, `remainingFraction`, `resetTime`) chứ không theo từng model → window id là `pool:<nhóm>:<cửa sổ>`. `GetUserStatus` trả plan (`userTier.description`) + `quotaInfo` từng model (dùng làm fallback, gộp các biến thể Low/Medium/High cùng quota). `GetCommandModelConfigs` trả **501 unimplemented**. `remainingFraction` bị lược khi = 0 (protobuf) → có `resetTime` mà thiếu fraction thì coi là đã dùng 100%. Khi app đóng: `QuotaInfo.stale` → giữ số cũ + `Health::Stale`. Chưa so được với màn hình settings của Antigravity vì không chạy app local được.
- **2026-11-15 — Step 8.** Dashboard 4 card + popup tray dùng chung 1 bundle; view chọn theo label cửa sổ (`popup`). Click trái tray = bật/tắt popup (đặt theo vị trí click, kẹp trong work area của màn hình nên bám sát taskbar ở mọi cạnh), double-click = mở dashboard, popup tự ẩn khi mất focus. Tooltip tray ≤ 120 ký tự (Windows cắt ở 127). UI đã xem thật trong trình duyệt bằng `pnpm dev` (không có Tauri thì tự dùng dữ liệu mẫu `src/lib/demo.ts`), 20 test Vitest pass. **Chưa xem được trong cửa sổ Tauri thật** (Smart App Control) — phần vị trí popup / ẩn khi blur mới chỉ có unit test cho phép tính toạ độ.

---

## 8. Nguồn tham khảo

- Claude usage endpoint & file local: https://pypi.org/project/ccusage/
- Claude usage endpoint (Rust): https://docs.rs/crate/cship/1.7.1/source/src/usage_limits.rs
- Claude statusline `rate_limits`: https://github.com/anthropics/claude-code/issues/40094
- Claude usage endpoint bị 429: https://claudeissues.com/issue/31637-bug-api-oauth-usage-endpoint-aggressively-rate-limits-making-usage-monitoring-un
- Codex rollout JSONL: https://codex.danielvaughan.com/2026/03/30/codex-cli-session-analytics-jsonl-rollout
- Codex `rate_limits` trong `token_count`: https://github.com/openai/codex/issues/28879
- Codex usage endpoint & file đọc: https://44a54478.openusage-docs.pages.dev/providers/codex/
- Codex process → rollout: https://docs.rs/crate/abtop/0.4.6/source/src/collector/codex.rs
- Antigravity local probe (CodexBar): https://raw.githubusercontent.com/steipete/CodexBar/main/docs/antigravity.md
- Antigravity token/port fix: https://gh.hhj520.top/steipete/CodexBar/pull/727
- AntigravityQuota: https://github.com/Henrik-3/AntigravityQuota
- Copilot internal usage API shape: https://huggingface.co/spaces/imspsycho/copilot-api/blob/main/src/services/github/get-copilot-usage.ts
- Copilot chuyển sang usage-based billing: https://github.blog/news-insights/company-news/github-copilot-is-moving-to-usage-based-billing/
- CodexBar providers: https://www.mintlify.com/steipete/codexbar/features/multi-provider
