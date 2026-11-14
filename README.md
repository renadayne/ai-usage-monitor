# AI Usage Monitor

App system tray cho **Windows 10/11 (x64)** theo dõi usage của các AI coding tool trên máy:
**Claude, Codex, Antigravity, GitHub Copilot**.

Với mỗi provider, app trả lời 3 câu hỏi:

- **Active?** — tool có đang chạy và đang tiêu quota hay không
- **Model?** — model đang/vừa được dùng gần nhất
- **Còn bao nhiêu?** — % quota đã dùng theo từng cửa sổ (5h, tuần, tháng…) và thời điểm reset

> **Trạng thái:** đang phát triển. Lộ trình và tiến độ từng step nằm trong [PLANNING.md](PLANNING.md).

## Stack

Tauri 2 (Rust) + React + TypeScript + Vite + Tailwind.

## Yêu cầu môi trường dev

- Rust, toolchain `stable-x86_64-pc-windows-msvc`
- Visual Studio Build Tools (workload "Desktop development with C++")
- Node.js LTS + pnpm
- WebView2 Runtime
- Git, GitHub CLI (`gh`)

## Lưu ý

- App chỉ **đọc** số liệu của chính tài khoản người dùng; không sửa file của tool khác, không refresh token, không gửi request inference, không telemetry.
- Nguồn quota đều là endpoint nội bộ / không có tài liệu chính thức nên có thể thay đổi bất kỳ lúc nào.

## License

[MIT](LICENSE)
