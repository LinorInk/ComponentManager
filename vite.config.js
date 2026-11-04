import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 期望前端 dev server 端口固定（tauri.conf.json 中 devUrl 一致）
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: "127.0.0.1",
    watch: {
      // src-tauri/target 是 Rust 编译产物，文件会被编译器独占锁定，
      // 不排除会导致 Vite 文件监听器崩溃（EBUSY）
      ignored: ["**/src-tauri/**", "**/node_modules/**", "**/dist/**"],
    },
  },
  build: {
    target: "es2021",
  },
});
