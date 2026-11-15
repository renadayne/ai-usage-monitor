# AI Usage Monitor

App system tray cho **Windows 10/11 (x64)** theo dõi usage của các AI coding tool trên máy:
**Claude, Codex, Antigravity, GitHub Copilot**.

Với mỗi provider, app trả lời 3 câu hỏi:

- **Active?** — tool có đang chạy và đang tiêu quota hay không
- **Model?** — model đang/vừa được dùng gần nhất
- **Còn bao nhiêu?** — % quota đã dùng theo từng cửa sổ (5h, tuần, tháng…) và thời điểm reset

> ⚠️ **Endpoint không chính thức.** Mọi nguồn quota đều là endpoint nội bộ / không có tài liệu
> chính thức của từng hãng. Chúng có thể đổi hoặc bị chặn bất kỳ lúc nào; khi đó provider tương
> ứng sẽ báo lỗi còn các provider khác vẫn chạy. Bạn tự cân nhắc điều khoản sử dụng của từng dịch vụ.

## Cài đặt

Tải file `AI Usage Monitor_x.y.z_x64-setup.exe` trong trang
[Releases](https://github.com/renadayne/ai-usage-monitor/releases) rồi chạy. Installer tự tải
WebView2 Runtime nếu máy chưa có.

Installer **chưa ký số**, nên:

- **SmartScreen** hiện "Windows protected your PC" → bấm **More info** → **Run anyway**.
- **Smart App Control** (Windows 11, nếu đang bật ở chế độ _On_) sẽ **chặn hẳn** app chưa ký và
  không có nút cho qua. Khi đó chỉ có 2 cách: tắt Smart App Control (Windows Security → App &
  browser control → Smart App Control; lưu ý tắt rồi thì không bật lại được nếu không cài lại
  Windows), hoặc tự ký bản build bằng chứng chỉ của bạn.

## Cách dùng

- App chạy nền ở khay hệ thống. **Click trái** vào icon: popup tóm tắt (mỗi provider một dòng với
  cửa sổ quota "căng" nhất). **Double-click** hoặc menu chuột phải → **Open Dashboard**: cửa sổ đầy đủ.
- Đóng cửa sổ dashboard = ẩn xuống khay. Thoát hẳn bằng menu chuột phải → **Quit**.
- Dashboard có 3 tab: **Overview** (4 card), **History** (biểu đồ 7 ngày), **Settings**.
- Màu thanh quota: dưới 70% bình thường, 70–90% cảnh báo (▲ High), trên 90% nguy cấp (⛔ Critical).

### Các trạng thái

| Hiển thị                    | Ý nghĩa                                                           |
| --------------------------- | ----------------------------------------------------------------- |
| Active / Idle / Not running | Đang tiêu quota / tool đang mở nhưng không hoạt động / không chạy |
| Stale                       | Đang hiện số cũ (ví dụ Antigravity đã đóng)                       |
| Sign-in required            | Không tìm thấy token, hoặc token hết hạn / bị từ chối             |
| Rate limited                | Provider đang giới hạn request; app tự chờ rồi thử lại            |
| Not installed               | Không tìm thấy tool trên máy                                      |
| (estimated)                 | Cửa sổ đã qua thời điểm reset mà chưa có số mới → tạm coi là 0%   |

### Settings

- Bật/tắt từng provider, chỉnh chu kỳ refresh. Mỗi provider có **sàn tối thiểu** để không bị
  rate-limit: Claude 300 giây, Codex 60, Antigravity 30, Copilot 300.
- Ngưỡng thông báo (mặc định 80% và 95%). Mỗi cửa sổ quota chỉ báo **1 lần** cho mỗi ngưỡng và
  1 lần khi quota reset.
- Khởi động cùng Windows (vào thẳng khay).
- Đường dẫn thủ công cho thư mục Claude Code / Codex, dùng khi tự dò không ra (ví dụ cài trong WSL).
- Kết nối GitHub cho Copilot (xem bên dưới).

Mọi thay đổi có hiệu lực ngay, không cần khởi động lại.

## Nguồn dữ liệu của từng provider

| Provider       | Quota                                                                                                                | Active / Model                                             | Đã kiểm chứng trên máy thật?                                          |
| -------------- | -------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- | --------------------------------------------------------------------- |
| Claude         | `GET api.anthropic.com/api/oauth/usage` bằng token trong `~/.claude/.credentials.json`                               | Log `~/.claude/projects/**/*.jsonl` + process `claude.exe` | Log: có. Usage endpoint: **chưa** (máy dev không có file credentials) |
| Codex          | `GET chatgpt.com/backend-api/wham/usage` bằng token trong `~/.codex/auth.json`; thêm `rate_limits` trong rollout log | Rollout `~/.codex/sessions/**` + process `codex.exe`       | Có (gói Plus)                                                         |
| Antigravity    | Language server chạy local của Antigravity (`127.0.0.1`, method `RetrieveUserQuotaSummary`, `GetUserStatus`)         | Process Antigravity + quota thay đổi giữa 2 lần đọc        | Có (gói Google AI Pro)                                                |
| GitHub Copilot | `GET api.github.com/copilot_internal/user`                                                                           | Process editor + quota thay đổi giữa 2 lần đọc             | **Chưa** (máy dev không có token dò được)                             |

Thư mục của Claude Code / Codex được dò theo thứ tự: đường dẫn trong Settings → biến môi trường
(`CLAUDE_CONFIG_DIR`, `CODEX_HOME`) → `%USERPROFILE%` → các distro WSL (`\\wsl.localhost\...`).

### Token GitHub cho Copilot

App thử lần lượt:

1. `%LOCALAPPDATA%\github-copilot\apps.json` / `hosts.json` (plugin Copilot của JetBrains, Vim… ghi ra).
2. `gh auth token` nếu có GitHub CLI.
3. Token do chính app lấy qua **GitHub device flow** (Settings → _Connect GitHub_). Token này lưu
   trong **Windows Credential Manager**, không ghi ra file. Device flow dùng client id công khai của
   plugin Copilot cho editor, nên màn hình xác nhận trên GitHub sẽ ghi tên "GitHub Copilot Plugin".

VS Code tự giữ token trong kho bí mật đã mã hoá của nó; app **không** đọc kho đó.

## Giới hạn đã biết

- **Claude**: nếu Claude Code chỉ chạy bên trong app Claude desktop thì không có
  `.credentials.json` → card hiện _Sign-in required_ (vẫn có Active và Model từ log). Đăng nhập
  bằng Claude Code CLI (`claude`, rồi `/login`) để có số quota. Token hết hạn thì app **không tự
  refresh** (tránh làm hỏng phiên của Claude Code) — mở Claude Code để nó tự làm mới.
  Usage endpoint rate-limit rất gắt nên không poll nhanh hơn 5 phút.
- **Codex**: bản Codex desktop hiện tại lưu rollout dạng `.jsonl.zst`; file nén lớn hơn 16 MB không
  được giải nén, khi đó model lấy từ `config.toml`. Đăng nhập bằng API key thì không có cửa sổ
  quota ("API key mode"). `chatgpt_base_url` trỏ tới host lạ thì app không gửi token đi.
- **Antigravity**: chỉ đọc được khi Antigravity đang mở; đóng app thì hiện số cuối cùng kèm nhãn
  _Stale_. Quota trả về theo **nhóm model dùng chung** (ví dụ "Gemini Models", "Claude and GPT
  models"), không theo từng model. Không biết model đang dùng.
- **Copilot**: shape response sau khi GitHub chuyển sang AI Credits (06/2026) **chưa được kiểm
  chứng**; parser nhận mọi thứ trông giống quota. Không biết model đang dùng. "Active" chỉ là suy
  đoán từ việc quota thay đổi.
- "Active" với tool cài trong **WSL** chỉ dựa vào thời điểm ghi log (Windows không thấy process trong WSL).
- Chưa đo được CPU/RAM khi chạy nền trên máy thật (xem mục Phát triển).

## Bảo mật và quyền riêng tư

- App chỉ **đọc**: không sửa file của tool khác, không refresh token thay tool khác, không gửi
  request inference, không telemetry.
- Token của tool nào chỉ được gửi tới đúng host gốc của tool đó (`api.anthropic.com`,
  `chatgpt.com`, `api.github.com` / `github.com`, `127.0.0.1`). Token chỉ nằm trong RAM; riêng
  token GitHub do app tự lấy thì nằm trong Windows Credential Manager.
- Kiểm tra chứng chỉ TLS chỉ bị tắt cho đúng `127.0.0.1` (language server của Antigravity dùng
  chứng chỉ tự ký).
- App không ghi log. Dữ liệu của app nằm trong `%LOCALAPPDATA%\com.renadayne.ai-usage-monitor\`:
  `usage.sqlite` (snapshot + lịch sử 30 ngày, chỉ có % và thời điểm), `settings.json`,
  `notify-state.json`.

## Xử lý sự cố

| Hiện tượng                                     | Cách xử lý                                                                                   |
| ---------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Card hiện _Not installed_ dù đã cài            | Nhập đường dẫn thư mục trong Settings (thường gặp khi cài trong WSL)                         |
| Claude / Codex hiện _Sign-in required_         | Mở tool đó và đăng nhập lại; app không tự làm mới token                                      |
| Claude hiện _Rate limited_ liên tục            | Bình thường với endpoint này; app tự giãn nhịp tới tối đa 30 phút. Đừng bấm Refresh liên tục |
| Antigravity hiện _Stale_                       | Mở Antigravity; trong vòng 1 phút số sẽ cập nhật                                             |
| Copilot hiện _Sign-in required_                | Settings → Connect GitHub, hoặc cài GitHub CLI rồi `gh auth login`                           |
| Một provider báo _Error_ sau khi hãng cập nhật | Endpoint có thể đã đổi; chạy script trong `scripts/probe/` để xem response mới và mở issue   |
| Không thấy thông báo                           | Kiểm tra Settings của app và Windows Settings → System → Notifications                       |

## Phát triển

Stack: Tauri 2 (Rust) + React + TypeScript + Vite + Tailwind. Lộ trình và nhật ký từng bước nằm
trong [PLANNING.md](PLANNING.md).

Yêu cầu: Rust (`stable-x86_64-pc-windows-msvc`), Visual Studio Build Tools (workload "Desktop
development with C++"), Node.js LTS + pnpm, WebView2 Runtime.

```
pnpm install
pnpm tauri dev          # chạy app
pnpm dev                # chỉ UI trong trình duyệt, dùng dữ liệu mẫu
pnpm lint && pnpm test  # frontend
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

- `AIUM_MOCK=1` chạy app với 4 provider giả.
- `scripts/probe/*.mjs` là các script spike: gọi nguồn dữ liệu thật **một lần** và ghi response đã
  redact vào `fixtures/`. Chạy lại khi nghi ngờ endpoint đổi shape.
- **Smart App Control**: nếu đang bật, Windows chặn mọi file thực thi chưa ký do `cargo` build ra
  (build script, `cargo clippy`, chính app), nên không build được Rust trên máy đó. Phần Rust của
  repo này được kiểm tra bằng GitHub Actions (`.github/workflows/ci.yml`).
- Phát hành: push tag `v*` → `.github/workflows/release.yml` build installer NSIS và tạo
  release.

## License

[MIT](LICENSE)
