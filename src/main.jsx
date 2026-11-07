import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

// 右键菜单默认禁用，避免弹出 WebView 自带的“刷新 / 检查”等浏览器菜单；
// 仅在输入框、文本域和可编辑区域保留系统菜单，方便复制粘贴。
document.addEventListener("contextmenu", (e) => {
  const el = e.target instanceof HTMLElement ? e.target : null;
  if (el && (el.closest("input, textarea") || el.isContentEditable)) return;
  e.preventDefault();
});

ReactDOM.createRoot(document.getElementById("root")).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
