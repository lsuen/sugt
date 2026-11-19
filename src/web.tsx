// 局域网 WebUI 入口。
// 复用桌面端同一份 UI 代码：main.tsx 末尾会自行挂载到 #root，
// transport 层按运行环境自动切换通道（浏览器走 /api/invoke，桌面走 Tauri IPC）。
// 因此这里只需引入 main.tsx，不做任何重复实现。
import './main';
